//! Real helper-process checks with isolated settings, private pipes and no provider requests.
#![cfg(feature = "runtime")]
use craft_ai_core::{Auth, Billing, Binding, Connection, ConnectionId, Locality, ModelId, RoleId, SecretRef, assistant::tool_model};
use craft_ai_native::storage::SharedStore;
use craft_assistant::{
    config::default_bridge_limits,
    protocol::{Event, Initialize},
};
use craft_assistant_bridge::{
    HelperMessage, HostMessage, PROTOCOL,
    wire::{Modifiers, PanelInput, read_message, write_message},
};
use std::{
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

struct Process {
    child: Child,
    input: Option<ChildStdin>,
    receive: mpsc::Receiver<Result<HelperMessage, String>>,
}
impl Process {
    fn start(root: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_craft-assistant"))
            .arg("--stdio")
            .env("CRAFT_AI_CONFIG_DIR", root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        let (tx, receive) = mpsc::sync_channel(8);
        std::thread::spawn(move || {
            loop {
                match read_message::<HelperMessage>(&mut output, default_bridge_limits().max_message_bytes) {
                    Ok(Some(event)) => {
                        if tx.send(Ok(event)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let _ = tx.send(Err(error));
                        break;
                    }
                }
            }
        });
        Self { child, input: Some(input), receive }
    }
    fn send(&mut self, message: HostMessage) {
        write_message(self.input.as_mut().unwrap(), &message, default_bridge_limits().max_message_bytes).unwrap();
    }
    fn initialize(&mut self, app: &str) {
        let config = Initialize { limits: default_bridge_limits(), translations: Default::default(), restore_chat: None };
        self.send(HostMessage::Initialize { protocol: PROTOCOL, app_id: app.into(), config: serde_json::to_value(config).unwrap() });
        self.until(|event| matches!(event, HelperMessage::Ready { protocol: PROTOCOL, .. }));
    }
    fn until(&self, mut predicate: impl FnMut(&HelperMessage) -> bool) -> HelperMessage {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "assistant process deadline expired");
            let event = self.receive.recv_timeout(remaining).expect("assistant event before deadline").expect("valid assistant pipe");
            if predicate(&event) {
                return event;
            }
        }
    }
    fn frame(&mut self) {
        self.send(HostMessage::Frame {
            input: PanelInput {
                size: [380.0, 640.0],
                pixels_per_point: 2.0,
                time: 1.0,
                focused: true,
                dark: true,
                palette: None,
                modifiers: Modifiers::default(),
                events: vec![],
            },
        });
    }
    fn command(&mut self, id: &str, name: &str, args: serde_json::Value) {
        self.send(HostMessage::Command { id: id.into(), name: name.into(), args });
    }
    fn result(&self, id: &str) -> Result<serde_json::Value, String> {
        let event=self.until(|event|matches!(event,HelperMessage::Event(value) if matches!(serde_json::from_value::<Event>(value.clone()),Ok(Event::Result{id:found,..}) if found==id)));
        let HelperMessage::Event(value) = event else { unreachable!() };
        let Event::Result { result, .. } = serde_json::from_value(value).unwrap() else { unreachable!() };
        result
    }
    fn model(&self, model: &str) {
        self.until(|event|matches!(event,HelperMessage::Event(value) if matches!(serde_json::from_value::<Event>(value.clone()),Ok(Event::Status(status)) if !status.settings_busy && status.model.contains(model))));
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn settings(root: &std::path::Path, model: &str) {
    let model = ModelId::new(model).unwrap();
    let connection = ConnectionId::new("suite-connection").unwrap();
    SharedStore::new(root.to_owned())
        .update(|store| {
            store.connections = vec![Connection {
                id: connection.clone(),
                label: "Suite test connection".into(),
                provider: "openai".into(),
                endpoint: Some("https://api.example.invalid/v1".into()),
                locality: Locality::Remote,
                auth: Auth::ApiKey { secret: SecretRef::new("one-shared-credential-reference").unwrap() },
                billing: Billing::Unknown,
                models: vec![tool_model(model.clone()).unwrap()],
                enabled: true,
                key_header: None,
            }];
            store.defaults = vec![Binding { role: RoleId::new("assistant.chat").unwrap(), connection, model }];
            Ok(())
        })
        .unwrap();
}
#[test]
fn actual_helper_renders_and_handles_settings_stop_and_unknown_commands() {
    let root = tempfile::tempdir().unwrap();
    let mut process = Process::start(root.path());
    process.initialize("pdfcraft");
    process.frame();
    let frame = process.until(|event| matches!(event, HelperMessage::Frame(_)));
    let HelperMessage::Frame(frame) = frame else { unreachable!() };
    assert!(!frame.meshes.is_empty());
    assert!(!frame.textures.is_empty());
    assert!(frame.accessibility.is_some());
    process.command("settings", "settings", serde_json::json!(true));
    assert!(process.result("settings").is_ok());
    process.command("stop", "stop", serde_json::json!({}));
    assert!(process.result("stop").is_ok());
    process.command("unknown", "bad-command", serde_json::json!({}));
    assert!(process.result("unknown").is_err());
    process.send(HostMessage::Shutdown);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = process.child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn three_app_processes_share_model_and_credential_reference_without_per_app_setup() {
    let root = tempfile::tempdir().unwrap();
    settings(root.path(), "suite-model-one");
    let mut processes = Vec::new();
    for app in ["pdfcraft", "wordcraft", "gridcraft"] {
        let mut process = Process::start(root.path());
        process.initialize(app);
        process.model("suite-model-one");
        processes.push(process);
    }
    settings(root.path(), "suite-model-two");
    for process in &mut processes {
        process.command("reload", "reload_settings", serde_json::json!({}));
        assert!(process.result("reload").is_ok());
        process.model("suite-model-two");
    }
    let store = SharedStore::new(root.path().to_owned()).load().unwrap();
    assert!(store.apps.is_empty());
    assert_eq!(store.connections.len(), 1);
    assert_eq!(store.defaults.len(), 1);
    assert_eq!(store.connections[0].auth.secret_ref().unwrap().as_str(), "one-shared-credential-reference");
}
#[test]
fn incompatible_protocol_exits_before_reading_settings() {
    let root = tempfile::tempdir().unwrap();
    let mut process = Process::start(root.path());
    process.send(HostMessage::Initialize { protocol: PROTOCOL + 1, app_id: "pdfcraft".into(), config: serde_json::json!({}) });
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = process.child.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn selected_file_grants_defer_component_replacement() {
    let root = tempfile::tempdir().unwrap();
    settings(root.path(), "suite-model");
    let file = root.path().join("synthetic-input.txt");
    std::fs::write(&file, "Synthetic assistant attachment.\n").unwrap();
    let mut process = Process::start(root.path());
    process.initialize("pdfcraft");
    process.model("suite-model");
    process.command("before", "restart_state", serde_json::Value::Null);
    assert_eq!(process.result("before").unwrap()["safe"], true);
    process.command("file", "add_file", serde_json::json!(file.to_string_lossy()));
    assert!(process.result("file").is_ok());
    process.command("after", "restart_state", serde_json::Value::Null);
    assert_eq!(process.result("after").unwrap()["safe"], false);
}
