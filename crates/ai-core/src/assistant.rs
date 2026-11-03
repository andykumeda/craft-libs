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
                Self::Chat => "Assistant model",
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
    /// MCP descriptor metadata is transported losslessly; providers use their supported fields.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub mcp_metadata: std::collections::BTreeMap<String, Value>,
}

impl ToolDefinition {
    pub fn from_mcp(value: &Value) -> Result<Self, AiError> {
        let object = value.as_object().ok_or_else(|| AiError::invalid("MCP tool descriptor must be an object"))?;
        let name = object.get("name").and_then(Value::as_str).ok_or_else(|| AiError::invalid("MCP tool descriptor needs a name"))?;
        let description =
            object.get("description").and_then(Value::as_str).ok_or_else(|| AiError::invalid("MCP tool descriptor needs a description"))?;
        let input_schema = object
            .get("inputSchema")
            .filter(|schema| schema.is_object())
            .ok_or_else(|| AiError::invalid("MCP tool descriptor needs an inputSchema"))?;
        Ok(Self {
            name: name.into(),
            description: description.into(),
            input_schema: input_schema.clone(),
            mcp_metadata: object
                .iter()
                .filter(|(key, _)| !["name", "description", "inputSchema"].contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        })
    }
    pub fn to_mcp(&self) -> Value {
        let mut object: serde_json::Map<String, Value> = self.mcp_metadata.iter().map(|(key, value)| (key.clone(), value.clone())).collect();
        object.insert("name".into(), Value::String(self.name.clone()));
        object.insert("description".into(), Value::String(self.description.clone()));
        object.insert("inputSchema".into(), self.input_schema.clone());
        Value::Object(object)
    }
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

#[cfg(test)]
mod mcp_descriptor_tests {
    use super::*;
    #[test]
    fn mcp_descriptor_metadata_and_legacy_wire_format_roundtrip() {
        let value = serde_json::json!({"name":"render_page","description":"Render","title":"Page image","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true},"outputSchema":{"type":"object"},"_meta":{"suite":"craft"}});
        let tool = ToolDefinition::from_mcp(&value).unwrap();
        assert_eq!(tool.to_mcp(), value);
        let wire = serde_json::to_value(&tool).unwrap();
        assert!(wire.get("input_schema").is_some());
        assert_eq!(serde_json::from_value::<ToolDefinition>(wire).unwrap(), tool);
        let old = serde_json::json!({"name":"old","description":"Old","input_schema":{"type":"object"}});
        assert!(serde_json::from_value::<ToolDefinition>(old).unwrap().mcp_metadata.is_empty());
    }
}
