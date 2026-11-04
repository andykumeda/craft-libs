use super::*;
use crate::config::SourceConfig;
#[cfg(unix)]
use serde_json::json;

fn configured(root: PathBuf) -> ComponentConfig {
    ComponentConfig {
        schema_version: 1,
        installation: craft_assistant_install::Config {
            root,
            publisher_team_id: "TESTTEAM".into(),
            signing_identifier: "org.craft.assistant.test".into(),
            local_certificate_sha1: None,
            limits: craft_assistant_install::Limits::default(),
        },
        source: SourceConfig { repo: "example/assistant".into(), asset_name: "assistant-macos-arm64.tar.gz".into() },
        bridge: default_bridge_limits(),
        startup_timeout_secs: 30,
    }
}

fn replacement() -> craft_assistant_install::Installed {
    craft_assistant_install::Installed {
        version: semver::Version::new(1, 1, 0),
        executable: PathBuf::from("/synthetic/new/craft-assistant"),
        bundle: PathBuf::from("/synthetic/new/CraftAssistant.app"),
        manifest: craft_assistant_install::Manifest {
            version: semver::Version::new(1, 1, 0),
            protocol: craft_assistant_install::Range { min: 1, max: 1 },
            settings: craft_assistant_install::Range { min: 1, max: 1 },
            history: craft_assistant_install::Range { min: 1, max: 1 },
        },
    }
}

#[test]
fn failure_cancels_pending_requests_and_never_replays() {
    let mut controller = Controller::new("pdfcraft", BTreeMap::new());
    controller.ready = true;
    controller.status.busy = true;
    controller.replacement = Some(replacement());
    controller.restart_barrier = Some("pending-snapshot".into());
    controller.requests.push(Request::BeginTurn { prompt: "synthetic edit".into(), files: Vec::new() });
    controller.fail("synthetic uncertain pipe outcome".into());
    assert!(!controller.ready());
    assert!(!controller.status.busy);
    assert!(controller.client.is_none());
    assert!(controller.replacement.is_none());
    assert!(controller.restart_barrier.is_none());
    assert!(matches!(controller.requests().as_slice(), [Request::Stop]));
    assert!(controller.requests().is_empty());
    assert!(controller.command("new_chat", Value::Null).is_err());
}

#[test]
fn startup_timeout_is_actionable_and_drops_pending_worker_connection() {
    let context = egui::Context::default();
    let mut controller = Controller::new("wordcraft", BTreeMap::new());
    let (_sender, receiver) = mpsc::sync_channel(1);
    controller.starting = Some(receiver);
    controller.timeout = Duration::from_secs(1);
    controller.started = Some(Instant::now() - Duration::from_secs(2));
    controller.poll(&context);
    assert!(controller.error().unwrap().contains("did not become ready"));
    assert!(controller.starting.is_none());
    assert!(matches!(controller.requests().as_slice(), [Request::Stop]));
}

#[test]
fn busy_turn_and_settings_worker_cannot_install_update() {
    let context = egui::Context::default();
    for settings in [false, true] {
        let mut controller = Controller::new("gridcraft", BTreeMap::new());
        // A broken idle guard would start a worker, but invalid local config still forbids network I/O.
        controller.config = Some(configured(PathBuf::from("relative-invalid-test-root")));
        controller.candidate = Some(Candidate {
            version: semver::Version::new(1, 1, 0),
            asset: craft_update::Asset {
                name: "assistant-macos-arm64.tar.gz".into(),
                size: 1,
                url: "https://github.com/example/assistant/releases/download/v1.1.0/assistant-macos-arm64.tar.gz".into(),
                sha256: Some("a".repeat(64)),
            },
        });
        controller.status.busy = !settings;
        controller.status.settings_busy = settings;
        controller.install_update(&context);
        assert!(controller.update.is_none());
    }
}

#[test]
fn refresh_does_not_replace_a_session_with_unsaved_draft_or_settings() {
    let context = egui::Context::default();
    let mut controller = Controller::new("pdfcraft", BTreeMap::new());
    controller.ready = true;
    controller.status.restart_safe = false;
    controller.status.chat_id = Some("synthetic-chat".into());
    controller.executable = Some(PathBuf::from("/synthetic/old/craft-assistant"));
    let (sender, receiver) = mpsc::sync_channel(1);
    sender
        .send(Ok(craft_assistant_install::Installed {
            version: semver::Version::new(1, 1, 0),
            executable: PathBuf::from("/synthetic/new/craft-assistant"),
            bundle: PathBuf::from("/synthetic/new/CraftAssistant.app"),
            manifest: craft_assistant_install::Manifest {
                version: semver::Version::new(1, 1, 0),
                protocol: craft_assistant_install::Range { min: 1, max: 1 },
                settings: craft_assistant_install::Range { min: 1, max: 1 },
                history: craft_assistant_install::Range { min: 1, max: 1 },
            },
        }))
        .unwrap();
    controller.refreshing = Some(receiver);
    controller.poll(&context);
    assert_eq!(controller.executable(), Some(std::path::Path::new("/synthetic/old/craft-assistant")));
    assert_eq!(controller.status.chat_id.as_deref(), Some("synthetic-chat"));
    assert!(controller.starting.is_none());
    assert!(controller.error().is_none());
}

#[test]
fn missing_component_keeps_configured_update_and_reload_controls() {
    let temporary = tempfile::tempdir().unwrap();
    let context = egui::Context::default();
    context.enable_accesskit();
    let mut controller = Controller::new("pdfcraft", BTreeMap::new());
    controller.config = Some(configured(temporary.path().join("installed")));
    controller.fail("The shared assistant is not installed.".into());
    let mut output = context.run_ui(egui::RawInput::default(), |ui| controller.show(ui));
    let accessibility = output.platform_output.accesskit_update.as_ref().unwrap();
    let labels: Vec<_> = accessibility.nodes.iter().filter_map(|(_, node)| node.label()).collect();
    assert!(labels.contains(&"Check for updates"), "{labels:?}");
    assert!(labels.contains(&"Reload component configuration"), "{labels:?}");
    output.textures_delta.clear();
}

#[cfg(unix)]
fn connected(messages: &[HelperMessage], expected: &str) -> (tempfile::TempDir, Controller) {
    use std::os::unix::fs::PermissionsExt;
    let temporary = tempfile::tempdir().unwrap();
    let executable = temporary.path().join("synthetic-helper");
    let mut bytes = Vec::new();
    for message in messages {
        craft_assistant_bridge::wire::write_message(&mut bytes, message, default_bridge_limits().max_message_bytes).unwrap();
    }
    let escaped = bytes.iter().map(|byte| format!("\\{byte:03o}")).collect::<String>();
    let script = format!("#!/bin/sh\nprintf '%b' '{escaped}'\nwhile IFS= read -r line; do :; done\n");
    std::fs::write(&executable, script).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut controller = Controller::new("pdfcraft", BTreeMap::new());
    controller.client = Some(Client::spawn(&executable, default_bridge_limits()).unwrap());
    controller.panel = Some(EmbeddedPanel::new(default_bridge_limits()).unwrap());
    controller.expected_version = Some(expected.into());
    controller.started = Some(Instant::now());
    (temporary, controller)
}

#[cfg(unix)]
fn ready(version: &str) -> HelperMessage {
    HelperMessage::Ready {
        protocol: PROTOCOL,
        version: version.into(),
        capabilities: vec!["tools-v1".into(), "render-v1".into(), "shared-settings-v1".into()],
    }
}

#[cfg(unix)]
fn await_outcome(controller: &mut Controller) {
    let context = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !controller.ready() && controller.error().is_none() && Instant::now() < deadline {
        controller.poll(&context);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(unix)]
#[test]
fn helper_ready_must_match_the_signed_manifest_version() {
    let (_temporary, mut controller) = connected(&[ready("2.0.0")], "1.0.0");
    await_outcome(&mut controller);
    assert!(!controller.ready());
    assert!(controller.error().unwrap().contains("incompatible"));
    assert!(matches!(controller.requests().as_slice(), [Request::Stop]));
}

#[cfg(unix)]
#[test]
fn service_events_before_handshake_are_refused() {
    let before = HelperMessage::Event(serde_json::to_value(Event::BeginTurn { prompt: "must not run".into(), files: Vec::new() }).unwrap());
    let (_temporary, mut controller) = connected(&[before, ready("1.0.0")], "1.0.0");
    await_outcome(&mut controller);
    assert!(!controller.ready());
    assert!(controller.error().unwrap().contains("before"));
    assert!(matches!(controller.requests().as_slice(), [Request::Stop]));
}

#[cfg(unix)]
#[test]
fn duplicate_handshake_is_refused_and_dynamic_translation_requests_are_forwarded() {
    let translate = HelperMessage::Event(json!({"kind":"translate","keys":["New chat"]}));
    let (_temporary, mut controller) = connected(&[ready("1.0.0"), translate], "1.0.0");
    await_outcome(&mut controller);
    let context = egui::Context::default();
    for _ in 0..20 {
        controller.poll(&context);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(controller.ready());
    assert!(controller.requests().iter().any(|request| matches!(request, Request::Translate{keys} if keys == &["New chat"])));
    let (_temporary, mut duplicate) = connected(&[ready("1.0.0"), ready("1.0.0")], "1.0.0");
    await_outcome(&mut duplicate);
    for _ in 0..20 {
        duplicate.poll(&context);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!duplicate.ready());
    assert!(duplicate.error().is_some());
}

#[cfg(unix)]
#[test]
fn fresh_unsafe_restart_barrier_retains_process_despite_stale_safe_status() {
    let snapshot = HelperMessage::Event(
        serde_json::to_value(Event::Result { id: "restart-snapshot".into(), result: Ok(json!({"safe":false,"chat_id":"synthetic-chat"})) }).unwrap(),
    );
    let (_temporary, mut controller) = connected(&[ready("1.0.0"), snapshot], "1.0.0");
    controller.status.restart_safe = true;
    controller.executable = Some(PathBuf::from("/synthetic/old/craft-assistant"));
    controller.replacement = Some(replacement());
    controller.restart_barrier = Some("restart-snapshot".into());
    await_outcome(&mut controller);
    let context = egui::Context::default();
    for _ in 0..20 {
        controller.poll(&context);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(controller.ready());
    assert!(controller.client.is_some());
    assert!(controller.replacement.is_none());
    assert!(controller.restart_barrier.is_none());
    assert!(controller.starting.is_none());
    assert!(controller.update_notice.as_deref().unwrap().contains("unsaved input"));
    assert_eq!(controller.executable(), Some(std::path::Path::new("/synthetic/old/craft-assistant")));
}

#[cfg(unix)]
#[test]
fn safe_restart_barrier_preserves_chat_for_pending_startup() {
    let snapshot = HelperMessage::Event(
        serde_json::to_value(Event::Result { id: "restart-snapshot".into(), result: Ok(json!({"safe":true,"chat_id":"synthetic-chat"})) }).unwrap(),
    );
    let (_temporary, mut controller) = connected(&[ready("1.0.0"), snapshot], "1.0.0");
    let (_startup, receiver) = mpsc::sync_channel(1);
    // Keep a synthetic launch pending so this unit test never resolves a user's real installation.
    controller.starting = Some(receiver);
    controller.replacement = Some(replacement());
    controller.restart_barrier = Some("restart-snapshot".into());
    let context = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(2);
    while controller.restore_chat.is_none() && controller.error().is_none() && Instant::now() < deadline {
        controller.poll(&context);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(controller.restore_chat.as_deref(), Some("synthetic-chat"));
    assert!(controller.starting.is_some());
    assert!(controller.client.is_none());
    assert!(controller.panel.is_none());
    assert!(!controller.ready());
    assert!(controller.replacement.is_none());
    assert!(controller.restart_barrier.is_none());
    assert!(controller.error().is_none());
}

#[cfg(unix)]
#[test]
fn pending_input_frame_prevents_restart_snapshot_until_acknowledged() {
    let (_temporary, mut controller) = connected(&[ready("1.0.0")], "1.0.0");
    await_outcome(&mut controller);
    let context = egui::Context::default();
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 400.0))), ..Default::default() };
    let mut output = context.run_ui(input, |ui| {
        controller.panel.as_mut().unwrap().show(ui, controller.client.as_mut().unwrap());
    });
    output.textures_delta.clear();
    assert!(controller.panel.as_ref().unwrap().input_pending());
    controller.replacement = Some(replacement());
    controller.restart_if_ready();
    assert!(controller.restart_barrier.is_none());
    assert!(controller.replacement.is_some());
    assert!(controller.client.is_some());
    assert!(controller.ready());
}

#[cfg(unix)]
#[test]
fn user_send_handoff_defers_replacement_before_app_starts_the_turn() {
    let begin =
        HelperMessage::Event(serde_json::to_value(Event::BeginTurn { prompt: "synthetic pending prompt".into(), files: Vec::new() }).unwrap());
    let (_temporary, mut controller) = connected(&[ready("1.0.0"), begin], "1.0.0");
    controller.status.restart_safe = true;
    controller.replacement = Some(replacement());
    await_outcome(&mut controller);
    let context = egui::Context::default();
    for _ in 0..20 {
        controller.poll(&context);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(controller.requests().iter().any(|request| matches!(request, Request::BeginTurn{prompt,..} if prompt == "synthetic pending prompt")));
    assert!(controller.restart_barrier.is_none(), "must not ask to restart during app handoff");
    assert!(controller.replacement.is_none(), "the app must remain able to start the authorized turn");
    assert!(controller.ready());
    assert!(controller.client.is_some());
}

#[cfg(unix)]
#[test]
fn a_hidden_panel_releases_keyboard_ownership_without_discarding_its_session() {
    let (_temporary, mut controller) = connected(&[ready("1.0.0")], "1.0.0");
    await_outcome(&mut controller);
    let context = egui::Context::default();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(380.0, 640.0))), ..Default::default() };
    let mut output = context.run_ui(raw, |ui| {
        let id = ui.scope_id().with("craft-embedded-assistant");
        ui.memory_mut(|memory| memory.request_focus(id));
        if let (Some(panel), Some(client)) = (&mut controller.panel, &mut controller.client) {
            panel.show(ui, client);
        }
    });
    output.textures_delta.clear();
    assert!(controller.owns_keyboard(), "the visible embedded panel received keyboard focus");
    controller.hidden();
    assert!(!controller.owns_keyboard(), "closing the panel must restore document keyboard shortcuts");
    assert!(controller.ready() && controller.client.is_some(), "hidden chats keep their helper session");
}

#[test]
fn host_restart_guard_preserves_active_work_and_missing_helper_is_safe() {
    let mut controller = Controller::new("wordcraft", BTreeMap::new());
    assert!(controller.restart_safe());
    controller.ready = true;
    assert!(!controller.restart_safe(), "ready helper has not confirmed its drafts are saved");
    controller.status.restart_safe = true;
    assert!(controller.restart_safe());
    controller.status.busy = true;
    assert!(!controller.restart_safe());
    controller.status.busy = false;
    controller.status.settings_busy = true;
    assert!(!controller.restart_safe());
    controller.status.settings_busy = false;
    controller.status.restart_safe = false;
    assert!(!controller.restart_safe(), "drafts and unsaved settings prevent restart");
    controller.status.restart_safe = true;
    controller.restart_barrier = Some("snapshot".into());
    assert!(!controller.restart_safe());
    controller.restart_barrier = None;
    let (_sender, receiver) = mpsc::channel();
    controller.update = Some(receiver);
    assert!(!controller.restart_safe());
    controller.update = None;
    let (_sender, receiver) = mpsc::channel();
    controller.starting = Some(receiver);
    assert!(!controller.restart_safe());
    controller.starting = None;
    controller.requests.push(Request::BeginTurn { prompt: "synthetic pending prompt".into(), files: vec![] });
    assert!(!controller.restart_safe());
    assert!(matches!(controller.requests.as_slice(), [Request::BeginTurn { .. }]), "the guard must not drain requests");
    controller.requests.clear();
    controller.requests.push(Request::ChooseFiles);
    assert!(!controller.restart_safe());
    controller.fail("synthetic failed helper".into());
    controller.status.settings_busy = true;
    assert!(controller.restart_safe(), "failed helper status cannot prevent ordinary app updates");
}

#[cfg(unix)]
#[test]
fn host_restart_guard_rejects_unacknowledged_panel_input_and_handshake() {
    let (_temporary, mut controller) = connected(&[ready("1.0.0")], "1.0.0");
    assert!(!controller.restart_safe(), "live transport still needs its handshake");
    await_outcome(&mut controller);
    controller.status.restart_safe = true;
    assert!(controller.restart_safe());
    let context = egui::Context::default();
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 400.0))), ..Default::default() };
    let mut output = context.run_ui(input, |ui| {
        controller.panel.as_mut().unwrap().show(ui, controller.client.as_mut().unwrap());
    });
    output.textures_delta.clear();
    assert!(controller.panel.as_ref().unwrap().input_pending());
    assert!(!controller.restart_safe(), "input can submit a turn before BeginTurn is received");
    assert!(controller.ready(), "the guard must not replace or stop the helper");
}

#[cfg(unix)]
#[test]
fn installed_component_updates_appear_only_in_settings() {
    let (temporary, mut controller) = connected(&[ready("0.1.0")], "0.1.0");
    let context = egui::Context::default();
    context.enable_accesskit();
    await_outcome(&mut controller);
    assert!(controller.ready());
    controller.config = Some(configured(temporary.path().join("installed")));
    controller.status.version = "0.1.1".into();
    controller.update_notice = Some("The shared assistant is up to date.".into());
    for open in [false, true, false] {
        controller.status.settings_open = open;
        let mut output = context.run_ui(egui::RawInput::default(), |ui| controller.show(ui));
        let accessibility = output.platform_output.accesskit_update.as_ref().unwrap();
        for label in ["Check for updates", "Shared assistant 0.1.1", "The shared assistant is up to date."] {
            assert_eq!(accessibility.nodes.iter().any(|(_, node)| node.label() == Some(label) || node.value() == Some(label)), open, "{label}");
        }
        output.textures_delta.clear();
    }
}
