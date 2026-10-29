use std::collections::VecDeque;
use std::sync::Mutex;

use craft_ai_core::assistant::{Message, MessageRole, RuntimeConfig, Target, ToolCall, ToolDefinition, tool_model};
use craft_ai_core::{AiError, AppId, Auth, Billing, Connection, ConnectionId, Locality, ModelId, ModelInfo};
use craft_ai_native::{Cancellation, ChatProvider, ModelChoice, ModelRequest, ToolHost, TurnEvent, TurnRequest, run_turn};
use serde_json::{Value, json};

struct Script {
    responses: Mutex<VecDeque<Message>>,
    models: Mutex<Vec<String>>,
}
impl ChatProvider for Script {
    fn complete(&self, request: &ModelRequest<'_>, cancel: &Cancellation, emit: &mut dyn FnMut(&str)) -> Result<Message, AiError> {
        cancel.check()?;
        self.models.lock().unwrap().push(request.model.to_string());
        let reply = self.responses.lock().unwrap().pop_front().ok_or_else(|| AiError::invalid("script exhausted"))?;
        emit(&reply.content);
        Ok(reply)
    }
    fn models(&self, _: &Connection, _: &RuntimeConfig, _: &Cancellation) -> Result<Vec<ModelInfo>, AiError> {
        Ok(Vec::new())
    }
}
struct Host {
    tool: &'static str,
    calls: usize,
    cancel_on_list: bool,
}
impl ToolHost for Host {
    fn initial_read(&self) -> Option<(String, Value)> {
        (self.tool == "inspect_document").then(|| (self.tool.into(), json!({})))
    }
    fn tools(&mut self, _: &Target, cancel: &Cancellation) -> Result<Vec<ToolDefinition>, AiError> {
        if self.cancel_on_list {
            cancel.cancel();
        }
        Ok(vec![ToolDefinition { name: self.tool.into(), description: "host operation".into(), input_schema: json!({"type":"object"}) }])
    }
    fn call(&mut self, target: &Target, call: &ToolCall, cancel: &Cancellation) -> Result<Value, AiError> {
        cancel.check()?;
        assert_eq!(target.id, "target-1");
        assert_eq!(call.name, self.tool);
        self.calls += 1;
        Ok(json!({"ok":true}))
    }
}
fn call(id: &str, name: &str, arguments: Value) -> Message {
    Message {
        role: MessageRole::Assistant,
        content: String::new(),
        tool_calls: vec![ToolCall { id: id.into(), name: name.into(), arguments }],
        tool_call_id: None,
    }
}
fn choice(model: &str) -> ModelChoice {
    let id = ModelId::new(model).unwrap();
    ModelChoice {
        connection: Connection {
            id: ConnectionId::new(model).unwrap(),
            label: model.into(),
            provider: "mock".into(),
            endpoint: Some("http://127.0.0.1:1/v1".into()),
            locality: Locality::Local,
            auth: Auth::None,
            billing: Billing::Free,
            models: vec![tool_model(id.clone()).unwrap()],
            enabled: true,
            key_header: None,
        },
        model: id,
    }
}
fn request() -> TurnRequest {
    TurnRequest {
        app: AppId::new("test-app").unwrap(),
        messages: vec![Message::text(MessageRole::User, "edit this")],
        target: Target { id: "target-1".into(), revision: 1 },
        chat: choice("chat-model"),
        control: choice("control-model"),
        config: RuntimeConfig::default(),
    }
}

#[test]
fn same_runtime_drives_two_independent_app_catalogs() {
    for tool in ["edit_document", "set_cell"] {
        let script = Script {
            responses: Mutex::new(VecDeque::from([
                call("delegate", "control_app", json!({"task":"make the requested edit"})),
                call("edit", tool, json!({})),
                Message::text(MessageRole::Assistant, "Edited and verified."),
                Message::text(MessageRole::Assistant, "Done."),
            ])),
            models: Mutex::new(Vec::new()),
        };
        let mut host = Host { tool, calls: 0, cancel_on_list: false };
        let result = run_turn(request(), &script, &mut host, &Cancellation::new(), |_| {}).unwrap();
        assert_eq!(host.calls, 1);
        assert_eq!(*script.models.lock().unwrap(), ["chat-model", "control-model", "control-model", "chat-model"]);
        assert_eq!(result.last().unwrap().content, "Done.");
        assert!(result.iter().any(|message| message.tool_call_id.as_deref() == Some("delegate")));
    }
}

#[test]
fn unrelated_chat_never_reads_the_host_catalog() {
    let script = Script { responses: Mutex::new(VecDeque::from([Message::text(MessageRole::Assistant, "Hello")])), models: Mutex::new(Vec::new()) };
    let mut host = Host { tool: "edit", calls: 0, cancel_on_list: true };
    let cancel = Cancellation::new();
    run_turn(request(), &script, &mut host, &cancel, |_| {}).unwrap();
    assert!(!cancel.is_cancelled());
    assert_eq!(host.calls, 0);
}

#[test]
fn cancellation_before_dispatch_prevents_edit() {
    let script =
        Script { responses: Mutex::new(VecDeque::from([call("delegate", "control_app", json!({"task":"edit"}))])), models: Mutex::new(Vec::new()) };
    let mut host = Host { tool: "edit", calls: 0, cancel_on_list: true };
    assert!(run_turn(request(), &script, &mut host, &Cancellation::new(), |_| {}).is_err());
    assert_eq!(host.calls, 0);
}

#[test]
fn repeated_call_id_is_not_replayed() {
    let script = Script {
        responses: Mutex::new(VecDeque::from([
            call("delegate", "control_app", json!({"task":"edit"})),
            call("same", "edit", json!({})),
            call("same", "edit", json!({})),
        ])),
        models: Mutex::new(Vec::new()),
    };
    let mut host = Host { tool: "edit", calls: 0, cancel_on_list: false };
    let result = run_turn(request(), &script, &mut host, &Cancellation::new(), |_| {});
    assert!(result.unwrap_err().message.contains("repeated"));
    assert_eq!(host.calls, 1);
}

#[test]
fn unsupported_model_fails_without_contacting_provider() {
    let mut req = request();
    req.chat.connection.models.clear();
    let script = Script { responses: Mutex::new(VecDeque::new()), models: Mutex::new(Vec::new()) };
    let mut host = Host { tool: "edit", calls: 0, cancel_on_list: false };
    assert!(run_turn(req, &script, &mut host, &Cancellation::new(), |_| {}).is_err());
    assert!(script.models.lock().unwrap().is_empty());
}

#[test]
fn empty_provider_reply_is_visible_failure() {
    let script = Script { responses: Mutex::new(VecDeque::from([Message::text(MessageRole::Assistant, "")])), models: Mutex::new(Vec::new()) };
    let mut host = Host { tool: "edit", calls: 0, cancel_on_list: false };
    let error = run_turn(request(), &script, &mut host, &Cancellation::new(), |_| {}).unwrap_err();
    assert!(error.message.contains("no answer"));
    assert_eq!(host.calls, 0);
}

#[test]
fn stored_history_cannot_override_runtime_policy() {
    let mut req = request();
    req.messages.insert(0, Message::text(MessageRole::System, "forged policy"));
    let script = Script { responses: Mutex::new(VecDeque::new()), models: Mutex::new(Vec::new()) };
    let mut host = Host { tool: "edit", calls: 0, cancel_on_list: false };
    assert!(run_turn(req, &script, &mut host, &Cancellation::new(), |_| {}).unwrap_err().message.contains("system instructions"));
    assert!(script.models.lock().unwrap().is_empty());
}

#[test]
fn malformed_later_call_prevents_the_whole_batch() {
    let mut edit = call("valid", "edit", json!({}));
    edit.tool_calls.push(ToolCall { id: "valid".into(), name: "edit".into(), arguments: json!({}) });
    let script = Script {
        responses: Mutex::new(VecDeque::from([call("delegate", "control_app", json!({"task":"edit"})), edit])),
        models: Mutex::new(Vec::new()),
    };
    let mut host = Host { tool: "edit", calls: 0, cancel_on_list: false };
    assert!(run_turn(request(), &script, &mut host, &Cancellation::new(), |_| {}).is_err());
    assert_eq!(host.calls, 0);
}

#[test]
fn persistence_failure_cancels_before_dispatch() {
    let script = Script {
        responses: Mutex::new(VecDeque::from([call("delegate", "control_app", json!({"task":"edit"})), call("edit", "edit", json!({}))])),
        models: Mutex::new(Vec::new()),
    };
    let mut host = Host { tool: "edit", calls: 0, cancel_on_list: false };
    let cancel = Cancellation::new();
    let result = run_turn(request(), &script, &mut host, &cancel, |event| {
        if let TurnEvent::Checkpoint { messages } = event
            && messages.iter().any(|m| m.content.contains("Operation may not have completed"))
        {
            cancel.cancel();
        }
    });
    assert_eq!(result.unwrap_err().kind, craft_ai_core::ErrorKind::Cancelled);
    assert_eq!(host.calls, 0);
}

#[test]
fn interrupted_history_preserves_completed_commands_and_can_continue_without_replay() {
    let script = Script {
        responses: Mutex::new(VecDeque::from([call("delegate", "control_app", json!({"task":"edit"})), call("edit", "edit", json!({}))])),
        models: Mutex::new(Vec::new()),
    };
    let mut host = Host { tool: "edit", calls: 0, cancel_on_list: false };
    let mut latest = Vec::new();
    assert!(
        run_turn(request(), &script, &mut host, &Cancellation::new(), |event| {
            if let TurnEvent::Checkpoint { messages } = event {
                latest = messages;
            }
        })
        .is_err()
    );
    let result: Value = serde_json::from_str(&latest.last().unwrap().content).unwrap();
    assert_eq!(result["activity"][0]["outcome"], "completed");
    assert_eq!(result["activity"][0]["call"]["id"], "edit");
    assert_eq!(host.calls, 1);
    latest.push(Message::text(MessageRole::User, "What happened?"));
    let mut req = request();
    req.messages = latest;
    let continuation = Script {
        responses: Mutex::new(VecDeque::from([Message::text(MessageRole::Assistant, "The edit completed.")])),
        models: Mutex::new(Vec::new()),
    };
    run_turn(req, &continuation, &mut host, &Cancellation::new(), |_| {}).unwrap();
    assert_eq!(host.calls, 1);
}

#[test]
fn uncertain_host_failure_is_recorded_and_never_retried() {
    struct Uncertain(usize);
    impl ToolHost for Uncertain {
        fn tools(&mut self, _: &Target, _: &Cancellation) -> Result<Vec<ToolDefinition>, AiError> {
            Ok(vec![ToolDefinition { name: "edit".into(), description: String::new(), input_schema: json!({"type":"object"}) }])
        }
        fn call(&mut self, _: &Target, _: &ToolCall, _: &Cancellation) -> Result<Value, AiError> {
            self.0 += 1;
            Err(AiError::new(craft_ai_core::ErrorKind::Network, "reply channel closed"))
        }
    }
    let script = Script {
        responses: Mutex::new(VecDeque::from([
            call("delegate", "control_app", json!({"task":"edit"})),
            call("edit", "edit", json!({})),
            call("different-id", "edit", json!({})),
        ])),
        models: Mutex::new(Vec::new()),
    };
    let mut host = Uncertain(0);
    let mut latest = Vec::new();
    assert_eq!(
        run_turn(request(), &script, &mut host, &Cancellation::new(), |event| {
            if let TurnEvent::Checkpoint { messages } = event {
                latest = messages;
            }
        })
        .unwrap_err()
        .kind,
        craft_ai_core::ErrorKind::Network
    );
    let result: Value = serde_json::from_str(&latest.last().unwrap().content).unwrap();
    assert_eq!(result["activity"][0]["outcome"], "unknown");
    assert_eq!(host.0, 1);
    assert_eq!(script.models.lock().unwrap().len(), 2);
}

#[test]
fn malformed_saved_tool_roles_are_rejected_before_request() {
    for mut bad in [Message::tool("unmatched", &json!({})), call("pending", "edit", json!({}))] {
        for user_role in [false, true] {
            if user_role {
                bad.role = MessageRole::User;
            }
            let mut req = request();
            req.messages.insert(0, bad.clone());
            let script = Script { responses: Mutex::new(VecDeque::new()), models: Mutex::new(Vec::new()) };
            let mut host = Host { tool: "edit", calls: 0, cancel_on_list: false };
            assert!(run_turn(req, &script, &mut host, &Cancellation::new(), |_| {}).is_err());
            assert!(script.models.lock().unwrap().is_empty());
        }
    }
}

#[test]
fn disabled_model_has_no_fallback() {
    let mut req = request();
    req.chat.connection.enabled = false;
    let script = Script { responses: Mutex::new(VecDeque::new()), models: Mutex::new(Vec::new()) };
    let mut host = Host { tool: "edit", calls: 0, cancel_on_list: false };
    assert!(run_turn(req, &script, &mut host, &Cancellation::new(), |_| {}).unwrap_err().message.contains("disabled"));
    assert!(script.models.lock().unwrap().is_empty());
}

#[test]
fn oversized_tool_result_keeps_completed_identity_in_recovery_history() {
    struct LargeResult;
    impl ToolHost for LargeResult {
        fn tools(&mut self, _: &Target, _: &Cancellation) -> Result<Vec<ToolDefinition>, AiError> {
            Ok(vec![ToolDefinition { name: "edit".into(), description: String::new(), input_schema: json!({"type":"object"}) }])
        }
        fn call(&mut self, _: &Target, _: &ToolCall, _: &Cancellation) -> Result<Value, AiError> {
            Ok(json!({"text":"x".repeat(8192)}))
        }
    }
    let mut req = request();
    req.config.max_context_bytes = 4096;
    let script = Script {
        responses: Mutex::new(VecDeque::from([call("delegate", "control_app", json!({"task":"edit"})), call("edit", "edit", json!({}))])),
        models: Mutex::new(Vec::new()),
    };
    let mut latest = Vec::new();
    assert!(
        run_turn(req, &script, &mut LargeResult, &Cancellation::new(), |event| {
            if let TurnEvent::Checkpoint { messages } = event {
                latest = messages;
            }
        })
        .unwrap_err()
        .message
        .contains("result exceeds")
    );
    let result: Value = serde_json::from_str(&latest.last().unwrap().content).unwrap();
    assert_eq!(result["activity"][0]["outcome"], "completed");
    assert_eq!(result["activity"][0]["result_omitted"], true);
    assert!(serde_json::to_vec(&latest).unwrap().len() <= 4096);
}

#[test]
fn tool_catalog_counts_toward_request_context_limit() {
    struct LargeCatalog;
    impl ToolHost for LargeCatalog {
        fn tools(&mut self, _: &Target, _: &Cancellation) -> Result<Vec<ToolDefinition>, AiError> {
            Ok(vec![ToolDefinition { name: "edit".into(), description: "x".repeat(8192), input_schema: json!({"type":"object"}) }])
        }
        fn call(&mut self, _: &Target, _: &ToolCall, _: &Cancellation) -> Result<Value, AiError> {
            panic!("must not dispatch")
        }
    }
    let mut req = request();
    req.config.max_context_bytes = 4096;
    let script =
        Script { responses: Mutex::new(VecDeque::from([call("delegate", "control_app", json!({"task":"edit"}))])), models: Mutex::new(Vec::new()) };
    assert!(run_turn(req, &script, &mut LargeCatalog, &Cancellation::new(), |_| {}).unwrap_err().message.contains("tool catalog"));
    assert_eq!(script.models.lock().unwrap().len(), 1);
}

#[test]
fn control_context_retains_prior_uncertain_tool_outcomes() {
    struct Inspect(Script);
    impl ChatProvider for Inspect {
        fn complete(&self, request: &ModelRequest<'_>, cancel: &Cancellation, emit: &mut dyn FnMut(&str)) -> Result<Message, AiError> {
            if request.model.as_str() == "control-model" {
                let context = serde_json::to_string(request.messages).unwrap();
                assert!(context.contains("uncertain-prior-edit"));
                assert!(context.contains("unknown"));
                assert!(request.messages.iter().all(|message| message.tool_calls.is_empty() && message.tool_call_id.is_none()));
            }
            self.0.complete(request, cancel, emit)
        }
        fn models(&self, _: &Connection, _: &RuntimeConfig, _: &Cancellation) -> Result<Vec<ModelInfo>, AiError> {
            Ok(Vec::new())
        }
    }
    let mut req = request();
    req.messages = vec![
        Message::text(MessageRole::User, "edit"),
        call("prior-delegation", "control_app", json!({"task":"edit"})),
        Message::tool("prior-delegation", &json!({"activity":[{"call":{"id":"uncertain-prior-edit"},"outcome":"unknown"}]})),
        Message::text(MessageRole::User, "Inspect what happened before continuing."),
    ];
    let script = Inspect(Script {
        responses: Mutex::new(VecDeque::from([
            call("delegate", "control_app", json!({"task":"inspect"})),
            Message::text(MessageRole::Assistant, "The earlier outcome is uncertain."),
            Message::text(MessageRole::Assistant, "Inspect the document first."),
        ])),
        models: Mutex::new(Vec::new()),
    });
    let mut host = Host { tool: "edit", calls: 0, cancel_on_list: false };
    run_turn(req, &script, &mut host, &Cancellation::new(), |_| {}).unwrap();
    assert_eq!(host.calls, 0);
}

#[test]
fn app_control_has_document_context_before_its_first_answer() {
    struct ContextProvider;
    impl ChatProvider for ContextProvider {
        fn complete(&self, request: &ModelRequest<'_>, _: &Cancellation, _: &mut dyn FnMut(&str)) -> Result<Message, AiError> {
            if request.model.as_str() == "control-model" {
                if request.messages.last().unwrap().tool_call_id.as_deref() == Some("provider-read") {
                    assert_eq!(request.messages.last().unwrap().role, MessageRole::Tool);
                    return Ok(Message::text(MessageRole::Assistant, "Document inspected."));
                }
                assert_eq!(
                    request.messages.last().unwrap().role,
                    MessageRole::User,
                    "host context must start a control request, not resume a provider callback"
                );
                assert!(
                    request.messages.iter().any(|message| message.role == MessageRole::Tool),
                    "app control must receive the initial document read before it can answer"
                );
                return Ok(call("provider-read", "inspect_document", json!({})));
            }
            if request.messages.iter().any(|message| message.role == MessageRole::Tool) {
                Ok(Message::text(MessageRole::Assistant, "Done."))
            } else {
                Ok(call("delegate", "control_app", json!({"task":"remove the dark background"})))
            }
        }
        fn models(&self, _: &Connection, _: &RuntimeConfig, _: &Cancellation) -> Result<Vec<ModelInfo>, AiError> {
            Ok(Vec::new())
        }
    }
    let mut host = Host { tool: "inspect_document", calls: 0, cancel_on_list: false };
    let mut req = request();
    req.messages = vec![Message::text(MessageRole::User, "remove the dark background")];
    let result = run_turn(req, &ContextProvider, &mut host, &Cancellation::new(), |_| {}).unwrap();
    assert_eq!(host.calls, 2);
    let result = result.iter().find(|message| message.role == MessageRole::Tool).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&result.content).unwrap()["activity"][0]["call"]["name"], "inspect_document");
}

/// Read-only live transport check; uses only synthetic host data, never an app document/history.
#[test]
#[ignore = "requires explicit CRAFT_AI_CONFIG_DIR with a signed-in Codex chat binding"]
fn live_codex_host_context_starts_and_continues() {
    use craft_ai_native::codex::{CodexOptions, CodexProvider};
    use craft_ai_native::storage::SharedStore;
    let root = std::path::PathBuf::from(std::env::var_os("CRAFT_AI_CONFIG_DIR").expect("set explicit live configuration"));
    assert!(root.is_absolute());
    let store = SharedStore::new(root.clone());
    let (chat, _) = craft_ai_native::resolve_choices(&store.load().unwrap(), &AppId::new("wordcraft").unwrap()).unwrap();
    assert_eq!(chat.connection.provider, "codex");
    let mut req = request();
    req.chat = chat.clone();
    req.control = chat;
    req.config = store.config().unwrap();
    req.messages = vec![Message::text(
        MessageRole::User,
        "Use control_app to inspect the bound synthetic document and report whether its ok flag is true. Do not modify anything.",
    )];
    let provider = CodexProvider::new(CodexOptions::in_shared_root(&root));
    let mut host = Host { tool: "inspect_document", calls: 0, cancel_on_list: false };
    let start = std::time::Instant::now();
    let mut control_used = false;
    let result = run_turn(req, &provider, &mut host, &Cancellation::new(), |event| {
        if let TurnEvent::Routed { role: craft_ai_core::assistant::AssistantRole::Control, .. } = event {
            control_used = true;
        }
    });
    eprintln!(
        "live Codex host-context probe: elapsed_secs={:.3} completed={} reads={} control_used={control_used}",
        start.elapsed().as_secs_f64(),
        result.is_ok(),
        host.calls
    );
    assert!(result.is_ok(), "{}", result.unwrap_err());
    assert!(host.calls >= 1 && control_used);
}
