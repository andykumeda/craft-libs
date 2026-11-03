//! Versioned assistant service messages inside the bridge's bounded JSON envelope.
use craft_ai_core::{
    AiError,
    assistant::{Target, ToolCall, ToolDefinition},
};
use craft_assistant_bridge::Limits;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Serialize, Deserialize)]
pub struct Initialize {
    pub limits: Limits,
    #[serde(default)]
    pub restore_chat: Option<String>,
    #[serde(default)]
    pub translations: BTreeMap<String, String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct StartTurn {
    pub prompt: String,
    pub target: Target,
    pub tools: Vec<ToolDefinition>,
    #[serde(default)]
    pub initial_read: Option<(String, Value)>,
    #[serde(default)]
    pub files: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ToolRequest {
    pub target: Target,
    pub call: ToolCall,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    BeginTurn { prompt: String, files: Vec<String> },
    Stop,
    ChooseFiles,
    Translate { keys: Vec<String> },
    Status(Status),
    Result { id: String, result: Result<Value, String> },
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Status {
    pub version: String,
    pub busy: bool,
    pub settings_busy: bool,
    #[serde(default = "legacy_settings_open")]
    pub settings_open: bool,
    #[serde(default)]
    pub restart_safe: bool,
    #[serde(default)]
    pub authorized_files: Vec<String>,
    pub chat_id: Option<String>,
    pub model: String,
    pub status: Option<String>,
    pub error: Option<String>,
}
pub type ToolResult = Result<Value, AiError>;

#[derive(Clone, Serialize, Deserialize)]
pub struct RestartState {
    pub safe: bool,
    pub chat_id: Option<String>,
}

// Older helpers cannot report their settings view; retain their update toolbar for recovery.
fn legacy_settings_open() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_status_keeps_updates_reachable_and_current_chat_hides_them() {
        let mut value = serde_json::to_value(Status::default()).unwrap();
        assert!(!serde_json::from_value::<Status>(value.clone()).unwrap().settings_open);
        value.as_object_mut().unwrap().remove("settings_open");
        assert!(serde_json::from_value::<Status>(value).unwrap().settings_open);
    }
}
