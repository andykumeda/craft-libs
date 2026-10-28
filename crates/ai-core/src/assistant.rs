//! Portable conversational assistant contracts. No I/O, UI, or application document types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AiError, CapabilityId, CapabilitySpec, Features, InputRole, ModelId, ModelInfo, Role, RoleId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantRole {
    Chat,
    Control,
}

impl AssistantRole {
    pub fn id(self) -> &'static str {
        match self {
            Self::Chat => "assistant.chat",
            Self::Control => "assistant.control",
        }
    }

    pub fn declaration(self) -> Result<Role, AiError> {
        Ok(Role {
            id: RoleId::new(self.id()).map_err(|e| AiError::invalid(e.to_string()))?,
            label: match self {
                Self::Chat => "General chat",
                Self::Control => "App control",
            }
            .into(),
            capability: CapabilityId::parse("text.complete").map_err(|e| AiError::invalid(e.to_string()))?,
            inputs: vec![InputRole::Prompt],
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn text(role: MessageRole, content: impl Into<String>) -> Self {
        Self { role, content: content.into(), tool_calls: Vec::new(), tool_call_id: None }
    }

    pub fn tool(id: impl Into<String>, result: &Value) -> Self {
        Self { role: MessageRole::Tool, content: result.to_string(), tool_calls: Vec::new(), tool_call_id: Some(id.into()) }
    }
}

/// The application alone interprets identity and revision; providers cannot change them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub id: String,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub interrupted: bool,
}

/// One source of operational defaults; persist overrides in the native shared configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    pub connect_timeout_secs: u64,
    pub request_timeout_secs: u64,
    pub max_rounds: u32,
    pub max_response_bytes: usize,
    pub max_context_bytes: usize,
    pub max_tools: usize,
    pub max_tool_arguments_bytes: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 15,
            request_timeout_secs: 180,
            max_rounds: 12,
            max_response_bytes: 8 * 1024 * 1024,
            max_context_bytes: 256 * 1024,
            max_tools: 128,
            max_tool_arguments_bytes: 64 * 1024,
        }
    }
}

impl RuntimeConfig {
    pub fn validate(&self) -> Result<(), AiError> {
        if !(1..=300).contains(&self.connect_timeout_secs)
            || !(1..=3600).contains(&self.request_timeout_secs)
            || !(1..=100).contains(&self.max_rounds)
            || !(1024..=64 * 1024 * 1024).contains(&self.max_response_bytes)
            || !(1024..=4 * 1024 * 1024).contains(&self.max_context_bytes)
            || !(1..=1024).contains(&self.max_tools)
            || !(256..=1024 * 1024).contains(&self.max_tool_arguments_bytes)
        {
            return Err(AiError::invalid("assistant limits are outside their supported ranges"));
        }
        Ok(())
    }
}

/// A manual model is explicitly asserted by the user to support streaming and tools.
pub fn tool_model(id: ModelId) -> Result<ModelInfo, AiError> {
    let capability = CapabilityId::parse("text.complete").map_err(|e| AiError::invalid(e.to_string()))?;
    Ok(ModelInfo::new(id.clone(), id.as_str()).with(CapabilitySpec::new(capability).accepting(InputRole::Prompt, 1).with_features(Features {
        streaming: true,
        tools: true,
        structured_output: false,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_results_preserve_call_identity() {
        let message = Message::tool("call-a", &serde_json::json!({"ok":true}));
        let json = serde_json::to_string(&message).unwrap();
        assert_eq!(serde_json::from_str::<Message>(&json).unwrap(), message);
        assert_eq!(message.tool_call_id.as_deref(), Some("call-a"));
    }

    #[test]
    fn hostile_limits_fail_without_panicking() {
        assert!(RuntimeConfig::default().validate().is_ok());
        assert!(RuntimeConfig { max_rounds: u32::MAX, ..Default::default() }.validate().is_err());
        assert!(RuntimeConfig { max_context_bytes: usize::MAX, ..Default::default() }.validate().is_err());
    }
}
