use crate::wire::{InputEvent, Modifiers, PanelInput};
use egui::{Event, ImeEvent, MouseWheelSource, MouseWheelUnit, PointerButton, TouchPhase};

impl From<egui::Modifiers> for Modifiers {
    fn from(v: egui::Modifiers) -> Self {
        Self { alt: v.alt, ctrl: v.ctrl, shift: v.shift, mac_cmd: v.mac_cmd, command: v.command }
    }
}
impl From<Modifiers> for egui::Modifiers {
    fn from(v: Modifiers) -> Self {
        Self { alt: v.alt, ctrl: v.ctrl, shift: v.shift, mac_cmd: v.mac_cmd, command: v.command }
    }
}
fn button(v: PointerButton) -> u8 {
    match v {
        PointerButton::Primary => 0,
        PointerButton::Secondary => 1,
        PointerButton::Middle => 2,
        PointerButton::Extra1 => 3,
        PointerButton::Extra2 => 4,
    }
}
fn phase(v: TouchPhase) -> u8 {
    match v {
        TouchPhase::Start => 0,
        TouchPhase::Move => 1,
        TouchPhase::End => 2,
        TouchPhase::Cancel => 3,
    }
}
/// Capture only events belonging to the panel; the caller tracks focus and pointer capture.
pub fn capture_input(ui: &egui::Ui, rect: egui::Rect, focused: bool, pointer_captured: bool) -> PanelInput {
    let hovered = ui.is_enabled() && ui.rect_contains_pointer(rect);
    let origin = rect.min.to_vec2();
    let mut events: Vec<InputEvent> = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|event| {
                let keyboard = focused && ui.is_enabled();
                let pointer = ui.is_enabled() && (hovered || pointer_captured);
                Some(match event {
                    Event::Text(text) if keyboard => InputEvent::Text { text: text.clone() },
                    Event::Paste(text) if keyboard => InputEvent::Paste { text: text.clone() },
                    Event::Copy if keyboard => InputEvent::Copy,
                    Event::Cut if keyboard => InputEvent::Cut,
                    Event::Key { key, physical_key, pressed, repeat, modifiers } if keyboard => InputEvent::Key {
                        key: key.name().into(),
                        physical: physical_key.map(|k| k.name().into()),
                        pressed: *pressed,
                        repeat: *repeat,
                        modifiers: (*modifiers).into(),
                    },
                    Event::ModifiersChanged(m) if keyboard || pointer => InputEvent::Modifiers { modifiers: (*m).into() },
                    Event::PointerMoved(pos) if pointer => InputEvent::PointerMoved { position: (*pos - origin).into() },
                    Event::PointerButton { pos, button: b, pressed, modifiers } if pointer => InputEvent::PointerButton {
                        position: (*pos - origin).into(),
                        button: button(*b),
                        pressed: *pressed,
                        modifiers: (*modifiers).into(),
                    },
                    Event::PointerGone => InputEvent::PointerGone,
                    Event::MouseWheel { unit, delta, phase: p, source, modifiers } if pointer => InputEvent::Wheel {
                        delta: (*delta).into(),
                        unit: match unit {
                            MouseWheelUnit::Point => 0,
                            MouseWheelUnit::Line => 1,
                            MouseWheelUnit::Page => 2,
                        },
                        phase: phase(*p),
                        source: match source {
                            MouseWheelSource::Unknown => 0,
                            MouseWheelSource::Wheel => 1,
                            MouseWheelSource::Trackpad => 2,
                            MouseWheelSource::Momentum => 3,
                        },
                        modifiers: (*modifiers).into(),
                    },
                    Event::Zoom(factor) if pointer => InputEvent::Zoom { factor: *factor },
                    Event::Rotate(radians) if pointer => InputEvent::Rotate { radians: *radians },
                    Event::Ime(ImeEvent::Preedit { text, active_range_chars }) if keyboard => {
                        InputEvent::ImePreedit { text: text.clone(), active_chars: active_range_chars.as_ref().map(|r| [r.start, r.end]) }
                    }
                    Event::Ime(ImeEvent::Commit(text)) if keyboard => InputEvent::ImeCommit { text: text.clone() },
                    Event::Ime(ImeEvent::DeleteSurrounding { before_chars, after_chars }) if keyboard => {
                        InputEvent::ImeDeleteChars { before: *before_chars, after: *after_chars }
                    }
                    Event::Ime(ImeEvent::DeleteSurroundingBytes { before_bytes, after_bytes }) if keyboard => {
                        InputEvent::ImeDeleteBytes { before: *before_bytes, after: *after_bytes }
                    }
                    Event::WindowFocused(f) => InputEvent::WindowFocused { focused: *f && focused },
                    _ => return None,
                })
            })
            .collect()
    });
    if !hovered && !pointer_captured {
        events.push(InputEvent::PointerGone);
    }
    PanelInput {
        size: rect.size().into(),
        pixels_per_point: ui.ctx().pixels_per_point(),
        time: ui.input(|i| i.time),
        focused: focused && ui.input(|i| i.focused),
        dark: ui.visuals().dark_mode,
        palette: Some(crate::wire::Palette {
            text: ui.visuals().text_color().to_array(),
            panel: ui.visuals().panel_fill.to_array(),
            window: ui.visuals().window_fill.to_array(),
            selection: ui.visuals().selection.bg_fill.to_array(),
            hyperlink: ui.visuals().hyperlink_color.to_array(),
            extreme_background: ui.visuals().extreme_bg_color.to_array(),
        }),
        modifiers: ui.input(|i| i.modifiers).into(),
        events,
    }
}
impl PanelInput {
    pub fn into_raw(self) -> Result<egui::RawInput, String> {
        let mut events = Vec::with_capacity(self.events.len().saturating_add(1));
        events.push(Event::ModifiersChanged(self.modifiers.into()));
        for event in self.events {
            events.push(match event {
                InputEvent::Text { text } => Event::Text(text),
                InputEvent::Paste { text } => Event::Paste(text),
                InputEvent::Copy => Event::Copy,
                InputEvent::Cut => Event::Cut,
                InputEvent::Key { key, physical, pressed, repeat, modifiers } => Event::Key {
                    key: egui::Key::from_name(&key).ok_or("unknown assistant key")?,
                    physical_key: physical.as_deref().map(|p| egui::Key::from_name(p).ok_or("unknown physical key")).transpose()?,
                    pressed,
                    repeat,
                    modifiers: modifiers.into(),
                },
                InputEvent::Modifiers { modifiers } => Event::ModifiersChanged(modifiers.into()),
                InputEvent::PointerMoved { position } => Event::PointerMoved(position.into()),
                InputEvent::PointerGone => Event::PointerGone,
                InputEvent::PointerButton { position, button, pressed, modifiers } => Event::PointerButton {
                    pos: position.into(),
                    button: match button {
                        0 => PointerButton::Primary,
                        1 => PointerButton::Secondary,
                        2 => PointerButton::Middle,
                        3 => PointerButton::Extra1,
                        4 => PointerButton::Extra2,
                        _ => return Err("unknown assistant pointer button".into()),
                    },
                    pressed,
                    modifiers: modifiers.into(),
                },
                InputEvent::Wheel { delta, unit, phase, source, modifiers } => Event::MouseWheel {
                    delta: delta.into(),
                    unit: match unit {
                        0 => MouseWheelUnit::Point,
                        1 => MouseWheelUnit::Line,
                        2 => MouseWheelUnit::Page,
                        _ => return Err("unknown assistant wheel unit".into()),
                    },
                    phase: match phase {
                        0 => TouchPhase::Start,
                        1 => TouchPhase::Move,
                        2 => TouchPhase::End,
                        3 => TouchPhase::Cancel,
                        _ => return Err("unknown assistant wheel phase".into()),
                    },
                    source: match source {
                        0 => MouseWheelSource::Unknown,
                        1 => MouseWheelSource::Wheel,
                        2 => MouseWheelSource::Trackpad,
                        3 => MouseWheelSource::Momentum,
                        _ => return Err("unknown assistant wheel source".into()),
                    },
                    modifiers: modifiers.into(),
                },
                InputEvent::Zoom { factor } => Event::Zoom(factor),
                InputEvent::Rotate { radians } => Event::Rotate(radians),
                InputEvent::ImePreedit { text, active_chars } => {
                    Event::Ime(ImeEvent::Preedit { text, active_range_chars: active_chars.map(|r| r[0]..r[1]) })
                }
                InputEvent::ImeCommit { text } => Event::Ime(ImeEvent::Commit(text)),
                InputEvent::ImeDeleteChars { before, after } => Event::Ime(ImeEvent::DeleteSurrounding { before_chars: before, after_chars: after }),
                InputEvent::ImeDeleteBytes { before, after } => {
                    Event::Ime(ImeEvent::DeleteSurroundingBytes { before_bytes: before, after_bytes: after })
                }
                InputEvent::WindowFocused { focused } => Event::WindowFocused(focused),
                InputEvent::ImeResetCompleted { .. } => continue,
                InputEvent::Accessibility { action } => Event::AccessKitActionRequest(serde_json::from_value(action).map_err(|e| e.to_string())?),
            });
        }
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size.into())),
            time: Some(self.time),
            focused: self.focused,
            system_theme: Some(if self.dark { egui::Theme::Dark } else { egui::Theme::Light }),
            events,
            ..Default::default()
        };
        if let Some(viewport) = raw.viewports.get_mut(&egui::ViewportId::ROOT) {
            viewport.native_pixels_per_point = Some(self.pixels_per_point);
        }
        Ok(raw)
    }
}
