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
    for (name, open) in [("pane", false), ("settings", true)] {
        pane.settings_open = open;
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(420.0, 900.0)).build_ui(|ui| {
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
