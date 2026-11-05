use super::*;
use craft_ai_core::assistant::tool_model;
use craft_ai_core::{AppId, Binding, ConnectionId, ModelId, RoleId, StoreFile};

fn configured() -> SettingsState {
    let mut draft = ConnectionDraft::from_preset(craft_ai_core::PRESETS.iter().find(|preset| preset.key == "ollama").unwrap());
    draft.models = vec![tool_model(ModelId::new("local-tools").unwrap()).unwrap()];
    let connection = draft.prepare(&[]).unwrap().connection;
    let bind = |role: AssistantRole| Binding {
        role: RoleId::new(role.id()).unwrap(),
        connection: connection.id.clone(),
        model: ModelId::new("local-tools").unwrap(),
    };
    let store = StoreFile {
        connections: vec![connection.clone()],
        defaults: vec![bind(AssistantRole::Chat), bind(AssistantRole::Control)],
        ..Default::default()
    };
    let mut settings = SettingsState::default();
    settings.set_store(store, Some(AppId::new("example-craft").unwrap()));
    settings
}

#[test]
fn submission_preserves_exact_text_and_disabled_drafts() {
    let mut state = State { draft: "  keep my whitespace\n".into(), ..Default::default() };
    assert!(state.submit().is_none());
    state.available = true;
    state.busy = true;
    assert!(state.submit().is_none());
    state.busy = false;
    assert!(matches!(state.submit(), Some(Action::Send(text)) if text == "  keep my whitespace\n"));
    assert!(state.draft.is_empty());
    state.draft = " \n\t ".into();
    assert!(state.submit().is_none());
    assert_eq!(state.draft, " \n\t ");
}

#[test]
fn saved_app_override_does_not_change_other_role_or_suite_default() {
    let mut settings = configured();
    let app = AppId::new("example-craft").unwrap();
    let binding = Binding {
        role: RoleId::new(AssistantRole::Control.id()).unwrap(),
        connection: ConnectionId::new("unavailable").unwrap(),
        model: ModelId::new("control-model").unwrap(),
    };
    settings.store.apps.entry(app.clone()).or_default().overrides.push(binding);
    settings.set_store(settings.store.clone(), Some(app));
    assert_eq!(settings.chat.model, "local-tools");
    assert!(!settings.chat.app_override);
    assert_eq!(settings.control.model, "control-model");
    assert!(settings.control.app_override);
    assert_eq!(settings.control.connection, "unavailable");
    assert!(settings.store.defaults.iter().all(|binding| binding.model.as_str() == "local-tools"));
}

#[test]
fn connection_validation_preserves_draft_until_host_acknowledges_save() {
    let mut settings = SettingsState::default();
    settings.connection.key = "synthetic-secret".into();
    assert!(settings.prepare_connection().is_none());
    assert_eq!(settings.connection.key, "synthetic-secret");
    assert!(settings.error.is_some());
    settings.connection.label = "Hosted test".into();
    settings.connection.endpoint = "https://example.com/v1".into();
    let action = settings.prepare_connection().unwrap();
    assert!(matches!(action, Action::SaveConnection(_)));
    assert_eq!(settings.connection.key, "synthetic-secret");
    assert!(!format!("{action:?}").contains("synthetic-secret"));
    assert!(settings.store.connections.is_empty());
}

#[test]
fn layout_keeps_composer_and_settings_inside_narrow_panel() {
    let ctx = egui::Context::default();
    for width in [280.0, 420.0, 700.0] {
        let mut state = State {
            available: true,
            chat_route: "Local / local-tools".into(),
            control_route: "Hosted / document-tools".into(),
            target_label: "Untitled document".into(),
            messages: vec![
                Message::text(MessageRole::User, "Summarize the selected text."),
                Message::text(MessageRole::Assistant, "Reading the selection…"),
            ],
            settings: configured(),
            ..Default::default()
        };
        for settings_open in [false, true] {
            state.settings_open = settings_open;
            let output = ctx.run_ui(
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 800.0))), ..Default::default() },
                |ui| {
                    let _ = show(ui, Id::unique("test-assistant"), &mut state, str::to_owned);
                    assert!(ui.min_rect().right() <= width + 1.0, "width {width}, settings {settings_open}: {:?}", ui.min_rect());
                },
            );
            output.drop_without_applying_deltas();
        }
    }
}

#[test]
fn drawing_busy_chat_never_issues_a_request_or_changes_history() {
    let ctx = egui::Context::default();
    let mut state = State {
        available: true,
        busy: true,
        draft: "next request".into(),
        active_chat: Some("one".into()),
        history: vec![ChatSummary { id: "one".into(), title: "Existing chat".into(), interrupted: true }],
        ..Default::default()
    };
    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        assert!(show(ui, Id::unique("busy-assistant"), &mut state, str::to_owned).is_empty());
    });
    output.drop_without_applying_deltas();
    assert_eq!(state.draft, "next request");
    assert_eq!(state.history.len(), 1);
    assert_eq!(state.active_chat.as_deref(), Some("one"));
}

#[test]
fn send_stop_and_settings_are_accessible_actions() {
    use egui_kittest::kittest::Queryable as _;
    struct View {
        pane: State,
        actions: Vec<Action>,
    }
    let state = View {
        pane: State { available: true, draft: "Please format the title".into(), settings: configured(), ..Default::default() },
        actions: Vec::new(),
    };
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(420.0, 800.0)).build_ui_state(
        |ui, view: &mut View| view.actions.extend(show(ui, Id::unique("accessible-assistant"), &mut view.pane, str::to_owned)),
        state,
    );
    harness.get_by_label("Send").click();
    harness.run();
    assert!(harness.state().actions.iter().any(|action| matches!(action, Action::Send(text) if text == "Please format the title")));
    harness.state_mut().pane.busy = true;
    harness.run();
    harness.get_by_label("Stop").click();
    harness.run();
    assert!(harness.state().actions.iter().any(|action| matches!(action, Action::Stop)));
    harness.get_by_label("Settings").click();
    harness.run();
    assert!(harness.state().pane.settings_open);
    harness.get_by_label("Stop").click();
    harness.run();
    assert_eq!(harness.state().actions.iter().filter(|action| matches!(action, Action::Stop)).count(), 2);
}

#[test]
#[ignore = "manual synthetic visual capture; writes PNGs to the temporary directory"]
fn export_widget_screenshots() {
    let mut pane = State {
        available: true,
        chat_route: "Local / local-tools".into(),
        control_route: "Hosted / document-tools".into(),
        target_label: "Sample document".into(),
        messages: vec![
            Message::text(MessageRole::User, "Make the heading bold and summarize the paragraph."),
            Message::text(MessageRole::Assistant, "The heading is bold. The paragraph describes the project's next steps."),
        ],
        activity: vec![Activity {
            label: "Document updated".into(),
            detail: "format.bold completed through MCP. Undo is available.".into(),
            failed: false,
        }],
        settings: configured(),
        ..Default::default()
    };
    for (name, open, dark, focused) in [
        ("pane", false, true, false),
        ("settings", true, true, false),
        ("pane-light", false, false, false),
        ("pane-focus", false, true, true),
        ("pane-light-focus", false, false, true),
    ] {
        pane.settings_open = open;
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(420.0, 900.0)).build_ui(|ui| {
            ui.ctx().set_visuals(composer_host_visuals(dark));
            if focused {
                ui.memory_mut(|memory| memory.request_focus(Id::unique("assistant-capture").with("composer")));
            }
            let _ = show(ui, Id::unique("assistant-capture"), &mut pane, str::to_owned);
        });
        harness.run();
        let path = std::env::temp_dir().join(format!("craft-assistant-{name}.png"));
        harness.render().unwrap().save(&path).unwrap();
        eprintln!("{}", path.display());
    }
}

#[test]
fn settings_operations_can_be_stopped_and_reloaded() {
    use egui_kittest::kittest::Queryable as _;
    struct View {
        state: SettingsState,
        actions: Vec<Action>,
    }
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 1500.0)).build_ui_state(
        |ui, view: &mut View| view.actions.extend(settings(ui, Id::unique("settings-actions"), &mut view.state, str::to_owned)),
        View { state: SettingsState { busy: true, ..Default::default() }, actions: Vec::new() },
    );
    harness.get_by_label("Stop").click();
    // Busy spinners intentionally keep requesting repaint; process the click without waiting
    // for a settled frame that only the host's completion/cancellation callback can produce.
    harness.run_steps(3);
    assert!(matches!(harness.state().actions.last(), Some(Action::Stop)));
    harness.state_mut().state.busy = false;
    harness.run();
    harness.get_by_label("Reload settings").click();
    harness.run();
    assert!(matches!(harness.state().actions.last(), Some(Action::ReloadSettings)));
}

fn long_title_pane(busy: bool) -> State {
    State {
        available: true,
        busy,
        active_chat: Some("long-title".into()),
        history: vec![ChatSummary {
            id: "long-title".into(),
            title: "Read the current document, make its heading bold, and explain which changes you made".into(),
            interrupted: false,
        }],
        chat_route: "Codex / selected-chat-model".into(),
        control_route: "Local / selected-control-model".into(),
        target_label: "Synthetic acceptance document".into(),
        status: busy.then(|| "Reading the current document…".into()),
        draft: "Make the next heading bold".into(),
        messages: vec![
            Message::text(MessageRole::User, "Read the current document, make its heading bold, and explain which changes you made."),
            Message::text(MessageRole::Assistant, "I am reading the current document before changing its heading."),
        ],
        ..Default::default()
    }
}

#[test]
fn long_history_titles_keep_the_header_and_composer_inside_the_pane() {
    use egui_kittest::kittest::Queryable as _;
    for width in [280.0, 420.0, 700.0] {
        for height in [480.0, 800.0] {
            for busy in [false, true] {
                let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(width, height)).build_ui_state(
                    |ui, pane| {
                        let _ = show(ui, Id::unique("long-title-assistant"), pane, str::to_owned);
                    },
                    long_title_pane(busy),
                );
                // Busy status includes a spinner, so it intentionally never settles.
                harness.run_steps(3);
                let chats = harness.get_by_label("Chat history").rect();
                let settings = harness.get_by_label("Settings").rect();
                let model = harness.get_by_label("Choose model").rect();
                let composer = harness.get_by_role(egui::Role::MultilineTextInput).rect();
                let send = harness.get_by_label(if busy { "Stop" } else { "Send" }).rect();
                let user = harness.get_by_label("Read the current document, make its heading bold, and explain which changes you made.").rect();
                assert!(chats.bottom() <= settings.bottom() + 1.0);
                assert!(composer.top() > chats.bottom());
                assert!(model.right() <= send.left(), "model and send overlap: {model:?} {send:?}");
                assert!(harness.query_by_label("Delete chat").is_none());
                for rect in [chats, settings, model, composer, send, user] {
                    assert!(
                        rect.left() >= 0.0 && rect.right() <= width + 0.5 && rect.bottom() <= height + 0.5,
                        "{width} x {height}, busy {busy}: control leaves the pane: {rect:?}"
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "manual synthetic visual capture; writes PNGs to the temporary directory"]
fn export_long_title_screenshots() {
    for width in [280.0, 420.0] {
        for busy in [false, true] {
            let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(width, 600.0)).build_ui_state(
                |ui, pane| {
                    let _ = show(ui, Id::unique("long-title-capture"), pane, str::to_owned);
                },
                long_title_pane(busy),
            );
            harness.run_steps(3);
            let phase = if busy { "busy" } else { "idle" };
            let path = std::env::temp_dir().join(format!("craft-assistant-long-title-{width}-{phase}.png"));
            harness.render().unwrap().save(&path).unwrap();
            eprintln!("{}", path.display());
        }
    }
}

#[test]
fn settings_focus_on_models_and_keep_connection_details_opt_in() {
    use egui_kittest::kittest::Queryable as _;
    struct View {
        settings: SettingsState,
        actions: Vec<Action>,
    }
    let mut state = configured();
    state.chat.app_override = true;
    let saved = state.store.clone();
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(420.0, 1000.0)).build_ui_state(
        |ui, view: &mut View| view.actions.extend(settings(ui, Id::unique("simple-settings"), &mut view.settings, str::to_owned)),
        View { settings: state, actions: Vec::new() },
    );
    assert!(harness.query_by_label("API base URL").is_none());
    assert!(harness.query_by_label("Custom model ID").is_none());
    harness.get_by_label("Apply for this app").click();
    harness.run();
    assert!(matches!(harness.state().actions.last(), Some(Action::SaveBinding {
        role: AssistantRole::Chat, app_override: true, model, ..
    }) if model == "local-tools"));
    assert_eq!(harness.state().settings.store, saved, "drawing/applying waits for host acknowledgement");
    harness.get_by_label("Add or edit a connection").click();
    harness.run();
    harness.get_by_label("API or local server").click();
    harness.run();
    assert!(harness.query_by_label("API base URL").is_some());
    assert!(harness.query_by_label("Delete connection").is_none());
    assert!(harness.query_by_label("Model to verify").is_none());
    harness.get_by_label("Check model tool support").click();
    harness.run();
    assert!(harness.query_by_label("Model to verify").is_some());
    assert_eq!(harness.state().settings.store, saved);
    assert_eq!(harness.state().actions.len(), 1, "expanding settings never sends a provider request");
}

#[test]
fn one_model_choice_handles_chat_and_documents_without_showing_legacy_roles() {
    use egui_kittest::kittest::Queryable as _;
    let mut settings = configured();
    settings.control.model = "legacy-control-model".into();
    let saved = settings.store.clone();
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(420.0, 900.0)).build_ui_state(
        |ui, pane| {
            let _ = show(ui, Id::unique("one-model-assistant"), pane, str::to_owned);
        },
        State {
            available: true,
            chat_route: "Local / local-tools".into(),
            control_route: "Legacy provider / legacy-control-model".into(),
            settings,
            ..Default::default()
        },
    );
    assert!(harness.query_by_label("Choose model").is_some());
    assert!(harness.query_by_label("General chat: Local / local-tools").is_none());
    assert!(harness.query_by_label("App control: Legacy provider / legacy-control-model").is_none());
    harness.get_by_label("Settings").click();
    harness.run();
    assert_eq!(harness.query_all_by_role_and_label(egui::Role::ComboBox, "Model").count(), 1);
    assert_eq!(harness.query_all_by_role_and_label(egui::Role::ComboBox, "Connection").count(), 1);
    assert!(harness.query_by_label("General chat").is_none());
    assert!(harness.query_by_label("App control").is_none());
    assert!(harness.query_by_label("Choose one model for chat and document work.").is_some());
    assert_eq!(harness.state().settings.control.model, "legacy-control-model");
    assert_eq!(harness.state().settings.store, saved, "hiding the legacy choice never mutates saved settings");
}

#[test]
fn compact_controls_preserve_draft_and_expose_files_and_model_settings() {
    use egui_kittest::kittest::Queryable as _;
    let pane = State { available: true, draft: "Keep this draft".into(), settings: configured(), ..Default::default() };
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(420.0, 800.0)).build_ui_state(
        |ui, pane| {
            let _ = show(ui, Id::unique("compact-controls"), pane, str::to_owned);
        },
        pane,
    );
    assert!(harness.query_by_label("Choose file…").is_none());
    assert!(harness.query_by_label("Assistant settings").is_none());
    harness.get_by_label("Add files").click();
    harness.run();
    assert!(harness.query_by_label("Choose file…").is_some());
    harness.get_by_label("Add files").click();
    harness.run();
    harness.get_by_label("Choose model").click();
    harness.run();
    assert!(harness.query_by_label("Changes affect this app only.").is_some());
    assert!(harness.query_by_label("Configure models in settings").is_some());
    assert_eq!(harness.state().draft, "Keep this draft");
}

#[test]
fn composer_model_choice_requests_validated_app_override_without_changing_saved_store() {
    use egui_kittest::kittest::Queryable as _;
    struct View {
        pane: State,
        actions: Vec<Action>,
    }
    let pane = State { available: true, draft: "Keep draft".into(), settings: configured(), ..Default::default() };
    let saved = pane.settings.store.clone();
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(420.0, 800.0)).build_ui_state(
        |ui, view: &mut View| view.actions.extend(show(ui, Id::unique("pick-model"), &mut view.pane, str::to_owned)),
        View { pane, actions: Vec::new() },
    );
    harness.get_by_label("Choose model").click();
    harness.run();
    harness.get_by_label("local-tools").click();
    harness.run();
    assert!(matches!(harness.state().actions.last(), Some(Action::SaveBinding {
        role: AssistantRole::Chat, model, app_override: true, manual_tools: false, ..
    }) if model == "local-tools"));
    assert_eq!(harness.state().pane.settings.store, saved);
    assert_eq!(harness.state().pane.draft, "Keep draft");
}

#[test]
fn new_chat_and_saved_history_have_explicit_separate_controls() {
    use egui_kittest::kittest::Queryable as _;
    struct View {
        pane: State,
        actions: Vec<Action>,
    }
    let pane = State {
        available: true,
        draft: "Keep my draft".into(),
        history: vec![ChatSummary { id: "saved".into(), title: "Saved conversation".into(), interrupted: false }],
        ..Default::default()
    };
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(280.0, 600.0)).build_ui_state(
        |ui, view: &mut View| view.actions.extend(show(ui, Id::unique("clear-chat-navigation"), &mut view.pane, str::to_owned)),
        View { pane, actions: Vec::new() },
    );
    assert!(harness.query_by_label("Chats").is_none());
    harness.get_by_label("New chat").click();
    harness.run();
    assert!(matches!(harness.state().actions.last(), Some(Action::NewChat)));
    assert_eq!(harness.state().pane.draft, "Keep my draft");
    harness.get_by_label("Chat history").click();
    harness.run();
    harness.get_by_label("Saved conversation").click();
    harness.run();
    assert!(matches!(harness.state().actions.last(), Some(Action::OpenChat(id)) if id=="saved"));
}

#[test]
fn composer_model_layout_stays_stable_between_frames() {
    use egui_kittest::kittest::Queryable as _;
    for width in [280.0, 338.0, 420.0] {
        let mut pane = long_title_pane(false);
        pane.chat_route = "Codex account / gpt-5.3-codex".into();
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(width, 600.0)).build_ui_state(
            |ui, pane| {
                let _ = show(ui, Id::unique("stable-model"), pane, str::to_owned);
            },
            pane,
        );
        harness.run_steps(4);
        let expected = harness.get_by_label("Choose model").rect();
        for frame in 0..12 {
            harness.run_steps(1);
            let actual = harness.get_by_label("Choose model").rect();
            assert_eq!(actual, expected, "width {width}, frame {frame}");
        }
    }
}

#[test]
fn transcript_hides_tool_messages_and_activity_but_keeps_conversation() {
    use egui_kittest::kittest::Queryable;
    let mut pane = State {
        messages: vec![
            Message::text(MessageRole::User, "Please edit"),
            Message::text(MessageRole::Assistant, "Done editing"),
            Message::text(MessageRole::Tool, "internal result"),
        ],
        activity: vec![Activity { label: "save_document".into(), detail: "internal activity".into(), failed: false }],
        ..Default::default()
    };
    let mut harness = egui_kittest::Harness::builder().build_ui(|ui| {
        let _ = show(ui, Id::unique("hidden-tools"), &mut pane, str::to_owned);
    });
    harness.run();
    assert!(harness.query_by_label("Please edit").is_some());
    assert!(harness.query_by_label("Done editing").is_some());
    assert!(harness.query_by_label("Tool result").is_none());
    assert!(harness.query_by_label("save_document").is_none());
}

fn composer_host_visuals(dark: bool) -> egui::Visuals {
    let mut visuals = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    // A host can use identical light fills, or dark fills that differ only slightly.
    visuals.panel_fill = if dark { egui::Color32::from_rgb(38, 38, 42) } else { egui::Color32::WHITE };
    visuals.extreme_bg_color = if dark { egui::Color32::from_rgb(30, 30, 34) } else { egui::Color32::WHITE };
    visuals
}

#[test]
fn composer_outline_is_visible_when_idle_and_focused_in_both_themes() {
    fn contrast(a: egui::Color32, b: egui::Color32) -> f32 {
        let luminance = |color: egui::Color32| {
            let channel = |value: u8| {
                let value = f32::from(value) / 255.0;
                if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
        };
        let a = luminance(a) + 0.05;
        let b = luminance(b) + 0.05;
        a.max(b) / a.min(b)
    }
    for dark in [false, true] {
        let mut idle_color = None;
        for focused in [false, true] {
            let ctx = egui::Context::default();
            let visuals = composer_host_visuals(dark);
            ctx.set_visuals(visuals.clone());
            let id = Id::unique("composer-outline-test");
            if focused {
                ctx.memory_mut(|memory| memory.request_focus(id.with("composer")));
            }
            let mut state = State { available: true, ..Default::default() };
            let output = ctx.run_ui(
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(360.0, 600.0))), ..Default::default() },
                |ui| {
                    let _ = show(ui, id, &mut state, str::to_owned);
                },
            );
            let outline = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Rect(rect)
                        if rect.corner_radius == egui::CornerRadius::same(12) && rect.fill == visuals.extreme_bg_color =>
                    {
                        Some(rect.stroke)
                    }
                    _ => None,
                })
                .expect("the composer must paint its rounded frame");
            assert!(outline.width >= 1.0 && outline.color.a() == 255, "dark={dark}, focused={focused}: {outline:?}");
            for background in [visuals.extreme_bg_color, visuals.panel_fill] {
                assert!(contrast(outline.color, background) >= 3.0, "dark={dark}, focused={focused}: {outline:?} against {background:?}");
            }
            if focused {
                assert_ne!(idle_color, Some(outline.color), "focus must remain visible");
            } else {
                idle_color = Some(outline.color);
            }
            output.drop_without_applying_deltas();
        }
    }
}
