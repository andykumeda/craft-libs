#[cfg(test)]
use crate::wire::Modifiers;
use crate::wire::{
    ACCESSIBILITY_SCHEMA, Accessibility, Ime, InputEvent, Limits, Mesh, PROTOCOL, PanelFrame, PanelInput, PlatformCommand, Surrounding, Texture,
    Vertex,
};
use crate::{Client, HelperMessage, HostMessage};
use egui::{Color32, Pos2, Rect, Vec2};
use std::collections::{HashMap, HashSet};
fn rect(v: Rect) -> [f32; 4] {
    [v.min.x, v.min.y, v.max.x, v.max.y]
}
fn to_rect(v: [f32; 4]) -> Rect {
    Rect::from_min_max(Pos2::new(v[0], v[1]), Pos2::new(v[2], v[3]))
}
fn texture_id(id: egui::TextureId) -> Result<u64, String> {
    match id {
        egui::TextureId::Managed(id) => Ok(id),
        egui::TextureId::User(_) => Err("assistant custom GPU textures are unsupported".into()),
    }
}

pub struct PanelRenderer {
    ctx: egui::Context,
    limits: Limits,
    resets: HashMap<u64, egui::ImeResetId>,
    focus_order: Vec<u64>,
    previous_focused: bool,
}
impl PanelRenderer {
    pub fn new(limits: Limits) -> Result<Self, String> {
        limits.validate()?;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        Ok(Self { ctx, limits, resets: HashMap::new(), focus_order: Vec::new(), previous_focused: false })
    }
    pub fn context(&self) -> &egui::Context {
        &self.ctx
    }
    pub fn frame(&mut self, mut input: PanelInput, mut draw: impl FnMut(&mut egui::Ui)) -> Result<PanelFrame, String> {
        input.validate(&self.limits)?;
        let tab_entry = input.focused
            && !self.previous_focused
            && input.events.iter().any(|event| matches!(event,InputEvent::Key{key,pressed:true,..} if key=="Tab"));
        if tab_entry && let Some(id) = self.ctx.memory(|memory| memory.focused()) {
            self.ctx.memory_mut(|memory| memory.surrender_focus(id));
        }
        let focused = self.ctx.memory(|m| m.focused()).map(|id| id.value());
        let focus_exit = input.events.iter().find_map(|event| match event {
            InputEvent::Key { key, pressed: true, modifiers, .. } if key == "Tab" && !modifiers.alt && !modifiers.ctrl && !modifiers.command => {
                if !modifiers.shift && focused.is_some() && focused == self.focus_order.last().copied() {
                    Some(true)
                } else if modifiers.shift && focused.is_some() && focused == self.focus_order.first().copied() {
                    Some(false)
                } else {
                    None
                }
            }
            _ => None,
        });
        if focus_exit.is_some() {
            input.events.retain(|e| !matches!(e,InputEvent::Key{key,pressed:true,..} if key=="Tab"));
            input.focused = false;
        }

        self.previous_focused = input.focused;
        let resets: Vec<_> = input
            .events
            .iter()
            .filter_map(|e| match e {
                InputEvent::ImeResetCompleted { serial } => self.resets.remove(serial),
                _ => None,
            })
            .collect();
        input.events.retain(|e| !matches!(e, InputEvent::ImeResetCompleted { .. }));
        let dark = input.dark;
        let palette = input.palette.clone();
        let mut raw = input.into_raw()?;
        raw.events.extend(resets.into_iter().map(|request_id| egui::Event::ImeResetCompleted { request_id }));
        let mut visuals = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        if let Some(p) = palette {
            visuals.override_text_color = Some(Color32::from_rgba_premultiplied(p.text[0], p.text[1], p.text[2], p.text[3]));
            visuals.panel_fill = Color32::from_rgba_premultiplied(p.panel[0], p.panel[1], p.panel[2], p.panel[3]);
            visuals.window_fill = Color32::from_rgba_premultiplied(p.window[0], p.window[1], p.window[2], p.window[3]);
            visuals.selection.bg_fill = Color32::from_rgba_premultiplied(p.selection[0], p.selection[1], p.selection[2], p.selection[3]);
            visuals.hyperlink_color = Color32::from_rgba_premultiplied(p.hyperlink[0], p.hyperlink[1], p.hyperlink[2], p.hyperlink[3]);
            visuals.extreme_bg_color =
                Color32::from_rgba_premultiplied(p.extreme_background[0], p.extreme_background[1], p.extreme_background[2], p.extreme_background[3]);
        }
        self.ctx.set_visuals(visuals);
        let mut output = self.ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.push_id("craft-independent-assistant", |ui| draw(ui));
            });
        });
        let result = (|| {
            let meshes = self
                .ctx
                .tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point)
                .into_iter()
                .map(|p| {
                    let egui::epaint::Primitive::Mesh(mesh) = p.primitive else { return Err("assistant GPU paint callbacks are unsupported".into()) };
                    Ok(Mesh {
                        clip: rect(p.clip_rect),
                        texture: texture_id(mesh.texture_id)?,
                        vertices: mesh
                            .vertices
                            .into_iter()
                            .map(|v| Vertex { position: v.pos.into(), uv: v.uv.into(), color: v.color.to_array() })
                            .collect(),
                        indices: mesh.indices,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let mut textures = Vec::new();
            for (id, deltas) in &output.textures_delta.set {
                for delta in deltas {
                    let egui::ImageData::Color(image) = &delta.image;
                    textures.push(Texture {
                        id: texture_id(*id)?,
                        size: image.size,
                        offset: delta.pos,
                        rgba: image.pixels.iter().flat_map(|c| c.to_array()).collect(),
                        magnify_linear: delta.options.magnification == egui::TextureFilter::Linear,
                        minify_linear: delta.options.minification == egui::TextureFilter::Linear,
                        wrap: match delta.options.wrap_mode {
                            egui::TextureWrapMode::ClampToEdge => 0,
                            egui::TextureWrapMode::Repeat => 1,
                            egui::TextureWrapMode::MirroredRepeat => 2,
                        },
                    });
                }
            }
            let free_textures = output.textures_delta.free.iter().map(|id| texture_id(*id)).collect::<Result<Vec<_>, _>>()?;
            let mut commands = Vec::new();
            for command in &output.platform_output.commands {
                commands.push(match command {
                    egui::OutputCommand::CopyText(text) => PlatformCommand::CopyText { text: text.clone() },
                    egui::OutputCommand::TextSelectionSettled(text) => PlatformCommand::Selection { text: text.clone() },
                    egui::OutputCommand::OpenUrl(url) => PlatformCommand::OpenUrl { url: url.url.clone() },
                    egui::OutputCommand::CopyImage(_) => return Err("assistant image clipboard output is unsupported".into()),
                });
            }
            let ime = output.platform_output.ime.as_ref().map(|ime| {
                if let Some(request) = ime.reset_request {
                    self.resets.insert(request.as_u64(), request);
                }
                Ime {
                    owner: ime.owner.value(),
                    generation: ime.context_generation,
                    purpose: match ime.purpose {
                        egui::IMEPurpose::Normal => 0,
                        egui::IMEPurpose::Password => 1,
                        egui::IMEPurpose::Terminal => 2,
                    },
                    rect: rect(ime.rect),
                    cursor: rect(ime.cursor_rect),
                    interrupt: ime.should_interrupt_composition,
                    reset_serial: ime.reset_request.map(|r| r.as_u64()),
                    surrounding: match &ime.surrounding_text {
                        egui::output::IMESurroundingText::Unavailable => None,
                        egui::output::IMESurroundingText::Available(s) => {
                            Some(Surrounding { text: s.text.clone(), cursor: s.cursor, anchor: s.anchor })
                        }
                    },
                    input_method_change: ime.change_cause == egui::ImeChangeCause::InputMethod,
                }
            });
            let accessibility = output
                .platform_output
                .accesskit_update
                .as_ref()
                .map(|a| {
                    Ok::<_, String>(Accessibility {
                        schema: ACCESSIBILITY_SCHEMA.into(),
                        root: a.tree.as_ref().map_or(egui::accesskit_root_id().value(), |t| t.root.0),
                        focus: a.focus.0,
                        nodes: a
                            .nodes
                            .iter()
                            .map(|(id, node)| serde_json::to_value(node).map(|v| (id.0, v)).map_err(|e| e.to_string()))
                            .collect::<Result<_, _>>()?,
                    })
                })
                .transpose()?;
            if let Some(tree) = &output.platform_output.accesskit_update {
                self.focus_order.clear();
                let nodes: HashMap<_, _> = tree.nodes.iter().map(|(id, node)| (*id, node)).collect();
                let mut stack = vec![tree.tree.as_ref().map_or(egui::accesskit_root_id().accesskit_id(), |tree| tree.root)];
                let mut seen = HashSet::new();
                while let Some(id) = stack.pop() {
                    if !seen.insert(id) {
                        continue;
                    }
                    if let Some(node) = nodes.get(&id) {
                        if node.supports_action(accesskit::Action::Focus) && !node.is_disabled() {
                            self.focus_order.push(id.0);
                        }
                        stack.extend(node.children().iter().rev().copied());
                    }
                }
            }
            let cursor = serde_json::to_value(output.platform_output.cursor_icon)
                .map_err(|e| e.to_string())?
                .as_str()
                .ok_or("invalid assistant cursor")?
                .to_owned();
            let frame = PanelFrame {
                focus_exit,
                meshes,
                textures,
                free_textures,
                cursor,
                ime,
                commands,
                accessibility,
                wants_keyboard: self.ctx.egui_wants_keyboard_input(),
                wants_pointer: self.ctx.egui_wants_pointer_input(),
            };
            frame.validate(&self.limits)?;
            Ok(frame)
        })();
        output.textures_delta.clear();
        result
    }
}

pub struct EmbeddedPanel {
    limits: Limits,
    frame: Option<PanelFrame>,
    textures: HashMap<u64, egui::TextureHandle>,
    texture_sizes: HashMap<u64, [usize; 2]>,
    pending_frees: Vec<u64>,
    events: Vec<HelperMessage>,
    frame_pending: bool,
    ready: bool,
    expected_version: Option<String>,
    focused: bool,
    pointer_captured: bool,
    error: Option<String>,
    pending_input: Vec<InputEvent>,
    reset: Option<(u64, egui::ImeResetId)>,
    ids: HashMap<accesskit::NodeId, u64>,
}
impl EmbeddedPanel {
    pub fn new(limits: Limits) -> Result<Self, String> {
        limits.validate()?;
        Ok(Self {
            limits,
            frame: None,
            textures: HashMap::new(),
            texture_sizes: HashMap::new(),
            pending_frees: Vec::new(),
            events: Vec::new(),
            frame_pending: false,
            ready: false,
            expected_version: None,
            focused: false,
            pointer_captured: false,
            error: None,
            pending_input: Vec::new(),
            reset: None,
            ids: HashMap::new(),
        })
    }
    pub fn poll_events(&mut self) -> Vec<HelperMessage> {
        std::mem::take(&mut self.events)
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn set_expected_version(&mut self, expected: Option<String>) {
        self.expected_version = expected;
    }
    pub fn input_pending(&self) -> bool {
        self.frame_pending || !self.pending_input.is_empty()
    }
    pub fn owns_keyboard(&self) -> bool {
        self.focused
    }
    pub fn poll(&mut self, ctx: &egui::Context, client: &mut Client) {
        self.poll_with_version_check(ctx, client, |_| Ok(()));
    }
    /// Validate the helper handshake before accepting any frame, even in the same batch.
    pub fn poll_with_version_check(&mut self, ctx: &egui::Context, client: &mut Client, check: impl Fn(&str) -> Result<(), String>) {
        for event in client.poll() {
            self.receive_checked(ctx, event, &check);
        }
    }
    fn receive_checked(&mut self, ctx: &egui::Context, event: HelperMessage, check: &impl Fn(&str) -> Result<(), String>) {
        if self.error.is_some() {
            return;
        }
        if let HelperMessage::Ready { version, .. } = &event
            && let Err(error) = check(version)
        {
            self.error = Some(error);
            return;
        }
        self.receive(ctx, event);
    }
    fn receive(&mut self, ctx: &egui::Context, event: HelperMessage) {
        match event {
            HelperMessage::Frame(frame) => {
                if !self.ready || self.error.is_some() {
                    self.error = Some("assistant rendered before a compatible handshake".into());
                    return;
                }
                self.frame_pending = false;
                if let Err(error) = self.accept(ctx, &frame) {
                    self.error = Some(error);
                } else {
                    self.frame = Some(*frame);
                }
            }
            HelperMessage::Ready { protocol, version, capabilities } => {
                if self.ready || protocol != PROTOCOL || self.expected_version.as_ref().is_some_and(|expected| expected != &version) {
                    self.error = Some("assistant handshake is incompatible or duplicated".into());
                } else {
                    self.ready = true;
                }
                self.events.push(HelperMessage::Ready { protocol, version, capabilities });
            }
            event => self.events.push(event),
        }
    }
    fn collect_platform_input(&mut self, ui: &egui::Ui, rect: Rect, input: &mut PanelInput) {
        if ui.is_enabled()
            && let Some((serial, request)) = self.reset
        {
            for event in ui.input(|i| i.raw.events.clone()) {
                if let egui::Event::ImeResetCompleted { request_id } = event
                    && request_id == request
                {
                    input.events.push(InputEvent::ImeResetCompleted { serial });
                    self.reset = None;
                }
            }
        }
        for event in ui.input(|i| i.raw.events.clone()) {
            if let egui::Event::AccessKitActionRequest(action) = event
                && ui.is_enabled()
                && let Some(action) = remote_action(action, &self.ids, rect.min.to_vec2(), ui.ctx().pixels_per_point())
            {
                if matches!(action.action, accesskit::Action::Focus | accesskit::Action::Click) {
                    self.focused = true;
                    input.focused = true;
                }
                if let Ok(action) = serde_json::to_value(action) {
                    input.events.push(InputEvent::Accessibility { action });
                }
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, client: &mut Client) {
        self.poll(ui.ctx(), client);
        if let Some(error) = client.error().or(self.error.as_deref()) {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }
        let rect = ui.available_rect_before_wrap();
        if rect.width() < 1.0 || rect.height() < 1.0 {
            return;
        }
        let id = ui.scope_id().with("craft-embedded-assistant");
        let exiting = self.frame.as_mut().and_then(|frame| frame.focus_exit.take());
        if let Some(forward) = exiting {
            self.focused = false;
            ui.ctx().memory_mut(|memory| {
                memory.request_focus(id);
                memory.move_focus(if forward { egui::FocusDirection::Next } else { egui::FocusDirection::Previous });
            });
        }
        let response = ui.interact(rect, id, egui::Sense::click_and_drag());
        ui.advance_cursor_after_rect(rect);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::GenericContainer, ui.is_enabled(), "Craft assistant"));
        if exiting.is_none() && response.has_focus() {
            self.focused = true;
        }
        if response.clicked() || response.drag_started() || (response.contains_pointer() && ui.input(|i| i.pointer.any_pressed())) {
            self.focused = true;
            response.request_focus();
        }
        if ui.input(|i| i.pointer.any_pressed()) && !response.contains_pointer() && !self.pointer_captured {
            self.focused = false;
        }
        let capture_pointer = self.pointer_captured || response.contains_pointer();
        self.pointer_captured = ui.input(|i| i.pointer.any_down()) && capture_pointer;
        let mut input = crate::capture_input(ui, rect, self.focused && ui.is_enabled(), capture_pointer && ui.is_enabled());
        if !ui.is_enabled() {
            input.events.clear();
        }
        {
            self.collect_platform_input(ui, rect, &mut input);
            self.pending_input.append(&mut input.events);
            if self.pending_input.len() > self.limits.max_events {
                self.error = Some("assistant input queue exceeded configured limit; reopen the panel".into());
                return;
            }
            if !self.frame_pending && (ui.is_enabled() || !self.pending_input.is_empty()) {
                input.events = std::mem::take(&mut self.pending_input);
                match client.send(HostMessage::Frame { input }) {
                    Ok(()) => self.frame_pending = true,
                    Err(error) => self.error = Some(error),
                }
            }
        }
        if self.focused && ui.is_enabled() {
            ui.input_mut(|i| {
                i.events.retain(|e| {
                    !matches!(
                        e,
                        egui::Event::Text(_)
                            | egui::Event::Paste(_)
                            | egui::Event::Copy
                            | egui::Event::Cut
                            | egui::Event::Key { .. }
                            | egui::Event::Ime(_)
                    )
                })
            });
        }
        if let Some(frame) = &self.frame {
            let origin = rect.min.to_vec2();
            for mesh in &frame.meshes {
                let Some(texture) = self.textures.get(&mesh.texture) else { continue };
                let paint = egui::epaint::Mesh {
                    texture_id: texture.id(),
                    indices: mesh.indices.clone(),
                    vertices: mesh
                        .vertices
                        .iter()
                        .map(|v| egui::epaint::Vertex {
                            pos: Pos2::from(v.position) + origin,
                            uv: v.uv.into(),
                            color: Color32::from_rgba_premultiplied(v.color[0], v.color[1], v.color[2], v.color[3]),
                        })
                        .collect(),
                };
                ui.painter().with_clip_rect(to_rect(mesh.clip).translate(origin).intersect(rect)).add(paint);
            }
            if response.contains_pointer()
                && let Ok(cursor) = serde_json::from_value::<egui::CursorIcon>(serde_json::Value::String(frame.cursor.clone()))
            {
                ui.ctx().set_cursor_icon(cursor);
            }
            if self.focused
                && ui.is_enabled()
                && let Some(ime) = &frame.ime
            {
                let owner = response.id.with(("remote-accessibility", ime.owner));
                let mut output = egui::output::IMEOutput::new(
                    owner,
                    ime.generation,
                    match ime.purpose {
                        1 => egui::IMEPurpose::Password,
                        2 => egui::IMEPurpose::Terminal,
                        _ => egui::IMEPurpose::Normal,
                    },
                    to_rect(ime.rect).translate(origin),
                    to_rect(ime.cursor).translate(origin),
                );
                output.should_interrupt_composition = ime.interrupt;
                if let Some(serial) = ime.reset_serial {
                    if self.reset.is_none_or(|(previous, _)| previous != serial) {
                        self.reset = ui.ctx().request_ime_reset(ui.ctx().viewport_id()).map(|request| (serial, request));
                    }
                    output.reset_request = self.reset.map(|(_, request)| request);
                }
                output.change_cause = if ime.input_method_change { egui::ImeChangeCause::InputMethod } else { egui::ImeChangeCause::Other };
                output.surrounding_text = ime.surrounding.as_ref().map_or(egui::output::IMESurroundingText::Unavailable, |s| {
                    egui::output::IMESurroundingText::from_text(&s.text, s.cursor, s.anchor, None)
                });
                ui.ctx().output_mut(|o| o.ime = Some(output));
            }
            if let Some(tree) = &frame.accessibility {
                self.ids.clear();
                if let Err(error) = graft(ui, response.id, origin, tree, &mut self.ids) {
                    self.error = Some(error);
                }
                if self.focused && ui.is_enabled() && tree.focus != tree.root {
                    let focus = response.id.with(("remote-accessibility", tree.focus));
                    let node = ui.ctx().accesskit_node_builder(focus, |node| node.clone());
                    let _ = ui.interact(rect, focus, egui::Sense::focusable_noninteractive());
                    if let Some(node) = node {
                        ui.ctx().accesskit_node_builder(focus, |dest| *dest = node);
                    }
                    ui.ctx().memory_mut(|memory| {
                        memory.request_focus(focus);
                        memory.set_focus_lock_filter(
                            focus,
                            egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
                        );
                    });
                }
            }
        }
        for id in std::mem::take(&mut self.pending_frees) {
            self.textures.remove(&id);
            self.texture_sizes.remove(&id);
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
    fn accept(&mut self, ctx: &egui::Context, frame: &PanelFrame) -> Result<(), String> {
        frame.validate(&self.limits)?;
        for id in std::mem::take(&mut self.pending_frees) {
            self.textures.remove(&id);
            self.texture_sizes.remove(&id);
        }
        for texture in &frame.textures {
            let options = egui::TextureOptions {
                magnification: if texture.magnify_linear { egui::TextureFilter::Linear } else { egui::TextureFilter::Nearest },
                minification: if texture.minify_linear { egui::TextureFilter::Linear } else { egui::TextureFilter::Nearest },
                wrap_mode: match texture.wrap {
                    0 => egui::TextureWrapMode::ClampToEdge,
                    1 => egui::TextureWrapMode::Repeat,
                    2 => egui::TextureWrapMode::MirroredRepeat,
                    _ => return Err("invalid assistant texture wrap".into()),
                },
                mipmap_mode: None,
            };
            let image = egui::ColorImage::from_rgba_premultiplied(texture.size, &texture.rgba);
            if let Some(offset) = texture.offset {
                let size = self.texture_sizes.get(&texture.id).ok_or("assistant texture patch has no base image")?;
                if offset[0].checked_add(texture.size[0]).is_none_or(|v| v > size[0])
                    || offset[1].checked_add(texture.size[1]).is_none_or(|v| v > size[1])
                {
                    return Err("assistant texture patch is outside its image".into());
                }
                self.textures.get_mut(&texture.id).ok_or("assistant texture is missing")?.set_partial(offset, image, options);
            } else {
                self.texture_sizes.insert(texture.id, texture.size);
                if self.texture_sizes.len() > 1024 {
                    return Err("assistant has too many live textures".into());
                }
                let total = self
                    .texture_sizes
                    .values()
                    .try_fold(0usize, |sum, size| sum.checked_add(size[0].checked_mul(size[1])?))
                    .ok_or("assistant texture total overflow")?;
                if total > self.limits.max_texture_pixels {
                    return Err("assistant textures exceed configured limit".into());
                }
                let handle = ctx.load_texture(format!("craft-assistant-{}", texture.id), image, options);
                self.textures.insert(texture.id, handle);
            }
        }
        if frame.meshes.iter().any(|mesh| !self.textures.contains_key(&mesh.texture)) {
            return Err("assistant mesh refers to a missing texture".into());
        }
        if frame.commands.iter().any(|command| matches!(command,PlatformCommand::OpenUrl{url} if !safe_web_url(url))) {
            return Err("assistant requested an invalid HTTP(S) URL".into());
        }
        for command in &frame.commands {
            match command {
                PlatformCommand::CopyText { text } => ctx.copy_text(text.clone()),
                PlatformCommand::Selection { text } => ctx.output_mut(|o| o.commands.push(egui::OutputCommand::TextSelectionSettled(text.clone()))),
                PlatformCommand::OpenUrl { url } => ctx.open_url(egui::OpenUrl::new_tab(url)),
            }
        }
        self.pending_frees = frame.free_textures.clone();
        Ok(())
    }
}
fn remote_action(
    mut action: accesskit::ActionRequest,
    ids: &HashMap<accesskit::NodeId, u64>,
    origin: Vec2,
    scale: f32,
) -> Option<accesskit::ActionRequest> {
    action.target_node = accesskit::NodeId(*ids.get(&action.target_node)?);
    action.target_tree = accesskit::TreeId::ROOT;
    match &mut action.data {
        Some(accesskit::ActionData::SetTextSelection(selection)) => {
            selection.anchor.node = accesskit::NodeId(*ids.get(&selection.anchor.node)?);
            selection.focus.node = accesskit::NodeId(*ids.get(&selection.focus.node)?);
        }
        Some(accesskit::ActionData::ScrollToPoint(point)) => {
            point.x -= f64::from(origin.x * scale);
            point.y -= f64::from(origin.y * scale);
        }
        _ => {}
    }
    Some(action)
}
fn safe_web_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")) else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    url.len() <= 2000 && !authority.is_empty() && !authority.contains('@') && !url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\\')
}
fn graft(ui: &egui::Ui, panel: egui::Id, origin: Vec2, tree: &Accessibility, ids: &mut HashMap<accesskit::NodeId, u64>) -> Result<(), String> {
    let root_id = egui::accesskit_root_id();
    let Some(previous) = ui.ctx().accesskit_node_builder(root_id, |node| node.children().to_vec()) else { return Ok(()) };
    let map = |raw: u64| if raw == tree.root { panel } else { panel.with(("remote-accessibility", raw)) };
    let node_id = |raw: accesskit::NodeId| map(raw.0).accesskit_id();
    let mut nodes = Vec::new();
    for (raw, value) in &tree.nodes {
        let mut node: accesskit::Node = serde_json::from_value(value.clone()).map_err(|e| format!("invalid assistant accessibility node: {e}"))?;
        let mapped = map(*raw);
        ids.insert(mapped.accesskit_id(), *raw);
        macro_rules! vectors{($($getter:ident=>$setter:ident),*)=>{$(node.$setter(node.$getter().iter().copied().map(node_id).collect::<Vec<_>>());)*}}
        vectors!(children=>set_children,controls=>set_controls,details=>set_details,described_by=>set_described_by,flow_to=>set_flow_to,labelled_by=>set_labelled_by,owns=>set_owns,radio_group=>set_radio_group);
        macro_rules! singles{($($getter:ident=>$setter:ident),*)=>{$(if let Some(id)=node.$getter(){node.$setter(node_id(id));})*}}
        singles!(active_descendant=>set_active_descendant,error_message=>set_error_message,in_page_link_target=>set_in_page_link_target,member_of=>set_member_of,next_on_line=>set_next_on_line,previous_on_line=>set_previous_on_line,popup_for=>set_popup_for);
        if let Some(mut selection) = node.text_selection().copied() {
            selection.anchor.node = node_id(selection.anchor.node);
            selection.focus.node = node_id(selection.focus.node);
            node.set_text_selection(selection);
        }
        if !ui.is_enabled() {
            node.set_disabled();
        }
        if *raw == tree.root {
            node.set_role(accesskit::Role::GenericContainer);
            node.set_transform(accesskit::Affine::translate((origin.x as f64, origin.y as f64)));
        }
        nodes.push((mapped, node));
    }
    let graph: HashMap<_, _> = nodes.iter().map(|(id, node)| (id.accesskit_id(), node.children())).collect();
    if graph.len() != nodes.len() || !graph.contains_key(&panel.accesskit_id()) {
        return Err("assistant accessibility tree has duplicate IDs or no root".into());
    }
    let mut seen = HashSet::new();
    let mut stack = vec![panel.accesskit_id()];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            return Err("assistant accessibility tree has a cycle or repeated child".into());
        }
        let children = graph.get(&id).ok_or("assistant accessibility child is missing")?;
        stack.extend(children.iter().copied());
    }
    if seen.len() != nodes.len() {
        return Err("assistant accessibility tree has disconnected nodes".into());
    }
    for (id, node) in nodes {
        ui.ctx().accesskit_node_builder(id, |dest| *dest = node);
    }
    // The node builders temporarily attach every remote node to the window; restore its actual children.
    ui.ctx().accesskit_node_builder(root_id, |root| root.set_children(previous));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_before_handshake_are_rejected_before_texture_allocation() {
        let limits = Limits { max_message_bytes: 1024, max_texture_pixels: 1024, max_vertices: 1024, max_events: 16 };
        let mut panel = EmbeddedPanel::new(limits).unwrap();
        panel.receive(&egui::Context::default(), HelperMessage::Frame(Box::default()));
        assert!(panel.error().is_some());
        assert!(panel.textures.is_empty());
        assert!(panel.frame.is_none());
    }
    #[test]
    fn wrong_release_ready_cannot_apply_following_frame_platform_commands() {
        let limits = Limits { max_message_bytes: 1024, max_texture_pixels: 1024, max_vertices: 1024, max_events: 16 };
        let mut panel = EmbeddedPanel::new(limits).unwrap();
        panel.set_expected_version(Some("1.0.0".into()));
        let ctx = egui::Context::default();
        panel.receive(&ctx, HelperMessage::Ready { protocol: PROTOCOL, version: "2.0.0".into(), capabilities: vec![] });
        let frame = PanelFrame { commands: vec![PlatformCommand::CopyText { text: "not applied".into() }], ..PanelFrame::default() };
        panel.receive(&ctx, HelperMessage::Frame(Box::new(frame)));
        assert!(panel.error().is_some());
        assert!(ctx.output(|o| o.commands.is_empty()));
        assert!(panel.frame.is_none());
    }
    #[test]
    fn minimum_version_rejection_prevents_texture_and_platform_effects_in_same_batch() {
        let mut panel = EmbeddedPanel::new(Limits { max_message_bytes: 1024, max_texture_pixels: 1024, max_vertices: 1024, max_events: 16 }).unwrap();
        let ctx = egui::Context::default();
        panel.receive_checked(&ctx, HelperMessage::Ready { protocol: PROTOCOL, version: "0.1.4".into(), capabilities: vec![] }, &|_| {
            Err("requires 0.1.5".into())
        });
        let frame = PanelFrame { commands: vec![PlatformCommand::CopyText { text: "not applied".into() }], ..PanelFrame::default() };
        panel.receive_checked(&ctx, HelperMessage::Frame(Box::new(frame)), &|_| Ok(()));
        assert_eq!(panel.error(), Some("requires 0.1.5"));
        assert!(panel.textures.is_empty());
        assert!(panel.frame.is_none());
        assert!(ctx.output(|o| o.commands.is_empty()));
    }
    #[test]
    fn tab_exits_helper_in_both_directions_at_keyboard_boundaries() {
        for forward in [true, false] {
            let limits = Limits { max_message_bytes: 8 * 1024 * 1024, max_texture_pixels: 4 * 1024 * 1024, max_vertices: 500_000, max_events: 256 };
            let mut renderer = PanelRenderer::new(limits).unwrap();
            let first = egui::Id::unique("boundary-first");
            let last = egui::Id::unique("boundary-last");
            let mut a = String::new();
            let mut b = String::new();
            let input = || PanelInput {
                size: [360.0, 600.0],
                pixels_per_point: 2.0,
                time: 1.0,
                focused: true,
                dark: true,
                palette: None,
                modifiers: Modifiers::default(),
                events: vec![],
            };
            renderer
                .frame(input(), |ui| {
                    ui.add(egui::TextEdit::singleline(&mut a).id(first));
                    ui.add(egui::TextEdit::singleline(&mut b).id(last));
                    ui.ctx().memory_mut(|m| m.request_focus(if forward { last } else { first }));
                })
                .unwrap();
            assert_eq!(renderer.focus_order.first(), Some(&first.value()));
            assert_eq!(renderer.focus_order.last(), Some(&last.value()));
            let mut next = input();
            next.time = 2.0;
            let modifiers = Modifiers { shift: !forward, ..Modifiers::default() };
            next.events.push(InputEvent::Key { key: "Tab".into(), physical: None, pressed: true, repeat: false, modifiers });
            let frame = renderer
                .frame(next, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut a).id(first));
                    ui.add(egui::TextEdit::singleline(&mut b).id(last));
                })
                .unwrap();
            assert_eq!(frame.focus_exit, Some(forward));
            let mut entry = input();
            entry.time = 3.0;
            entry.events.push(InputEvent::Key { key: "Tab".into(), physical: None, pressed: true, repeat: false, modifiers: Modifiers::default() });
            let frame = renderer
                .frame(entry, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut a).id(first));
                    ui.add(egui::TextEdit::singleline(&mut b).id(last));
                })
                .unwrap();
            assert_eq!(frame.focus_exit, None);
            assert_eq!(renderer.context().memory(|memory| memory.focused()), Some(first));
        }
    }
    #[test]
    fn accessible_text_selection_maps_both_endpoints_back_to_helper() {
        let ids = HashMap::from([(accesskit::NodeId(10), 1), (accesskit::NodeId(20), 2)]);
        let action = accesskit::ActionRequest {
            action: accesskit::Action::SetTextSelection,
            target_tree: accesskit::TreeId::ROOT,
            target_node: accesskit::NodeId(10),
            data: Some(accesskit::ActionData::SetTextSelection(accesskit::TextSelection {
                anchor: accesskit::TextPosition { node: accesskit::NodeId(10), character_index: 0 },
                focus: accesskit::TextPosition { node: accesskit::NodeId(20), character_index: 2 },
            })),
        };
        let remote = remote_action(action, &ids, Vec2::ZERO, 2.0).unwrap();
        assert_eq!(remote.target_node, accesskit::NodeId(1));
        let Some(accesskit::ActionData::SetTextSelection(selection)) = remote.data else { panic!("text selection") };
        assert_eq!(selection.anchor.node, accesskit::NodeId(1));
        assert_eq!(selection.focus.node, accesskit::NodeId(2));
    }
    #[test]
    fn web_urls_require_authority_and_reject_native_opener_ambiguities() {
        assert!(safe_web_url("https://example.test/login?code=one&state=two"));
        for url in ["file:///private/tmp/x", "https://", "https://user@example.test/", "https://example.test/\n", "https://example.test\\evil"] {
            assert!(!safe_web_url(url));
        }
    }
    #[test]
    fn graft_keeps_host_siblings_and_remaps_all_remote_relations() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut root = accesskit::Node::new(accesskit::Role::Window);
        root.set_children([accesskit::NodeId(2), accesskit::NodeId(3)]);
        let mut text = accesskit::Node::new(accesskit::Role::TextInput);
        text.set_label("Remote composer");
        text.set_labelled_by([accesskit::NodeId(3)]);
        let label = accesskit::Node::new(accesskit::Role::Label);
        let tree = Accessibility {
            schema: ACCESSIBILITY_SCHEMA.into(),
            root: 1,
            focus: 2,
            nodes: vec![
                (1, serde_json::to_value(root).unwrap()),
                (2, serde_json::to_value(text).unwrap()),
                (3, serde_json::to_value(label).unwrap()),
            ],
        };
        let panel = egui::Id::unique("bridge-panel");
        let mut ids = HashMap::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label("Host sibling");
            let response = ui.interact(Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(300.0, 500.0)), panel, egui::Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::GenericContainer, true, "Assistant"));
            graft(ui, panel, Vec2::new(10.0, 10.0), &tree, &mut ids).unwrap();
        });
        let access = output.platform_output.accesskit_update.as_ref().unwrap();
        let remote = panel.with(("remote-accessibility", 2u64)).accesskit_id();
        let parent = access.nodes.iter().find(|(id, _)| *id == panel.accesskit_id()).unwrap();
        assert_eq!(parent.1.children(), &[remote, panel.with(("remote-accessibility", 3u64)).accesskit_id()]);
        let root = access.nodes.iter().find(|(id, _)| *id == egui::accesskit_root_id().accesskit_id()).unwrap();
        assert!(access.nodes.iter().any(|(_, node)| node.children().contains(&panel.accesskit_id())));
        assert!(!root.1.children().contains(&remote));
        let text = access.nodes.iter().find(|(id, _)| *id == remote).unwrap();
        assert_eq!(text.1.labelled_by(), &[panel.with(("remote-accessibility", 3u64)).accesskit_id()]);
        assert_eq!(ids.get(&remote), Some(&2));
        output.textures_delta.clear();
    }
}

#[cfg(test)]
mod actual_tree_tests {
    use super::*;
    #[test]
    fn real_helper_tree_grafts_into_host_and_preserves_sibling() {
        let limits = Limits { max_message_bytes: 8 * 1024 * 1024, max_texture_pixels: 4 * 1024 * 1024, max_vertices: 1_000_000, max_events: 256 };
        let mut renderer = PanelRenderer::new(limits).unwrap();
        let mut text = "remote".to_string();
        let frame = renderer
            .frame(
                PanelInput {
                    size: [360.0, 600.0],
                    pixels_per_point: 2.0,
                    time: 1.0,
                    focused: true,
                    dark: true,
                    palette: None,
                    modifiers: Modifiers::default(),
                    events: vec![],
                },
                |ui| {
                    ui.label("Assistant");
                    ui.add(egui::TextEdit::singleline(&mut text));
                    ui.button("Send").clicked();
                },
            )
            .unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut ids = HashMap::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label("Document tools");
            let panel = ui.allocate_response(Vec2::new(360.0, 600.0), egui::Sense::click());
            panel.widget_info(|| egui::WidgetInfo::labeled(egui::Role::GenericContainer, true, "Assistant"));
            graft(ui, panel.id, panel.rect.min.to_vec2(), frame.accessibility.as_ref().unwrap(), &mut ids).unwrap();
        });
        let tree = output.platform_output.accesskit_update.as_ref().unwrap();
        assert!(tree.nodes.iter().any(|(_, n)| n.label() == Some("Document tools") || n.value() == Some("Document tools")));
        assert!(tree.nodes.iter().any(|(_, n)| n.role() == accesskit::Role::TextInput));
        output.textures_delta.clear();
    }
}
