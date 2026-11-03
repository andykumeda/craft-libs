//! Provider adaptation for inline MCP images; canonical tool results and history stay untouched.
use base64::Engine;
use craft_ai_core::AiError;
use craft_ai_core::assistant::{Message, MessageRole, RuntimeConfig};
use serde_json::Value;

pub(crate) struct Image {
    pub data_uri: String,
    pub bytes: usize,
}
pub(crate) struct ToolContent {
    pub text: String,
    pub images: Vec<Image>,
}

pub(crate) fn split(message: &Message, max_bytes: usize) -> Result<ToolContent, AiError> {
    let unchanged = || ToolContent { text: message.content.clone(), images: Vec::new() };
    if message.role != MessageRole::Tool {
        return Ok(unchanged());
    }
    let mut value = match serde_json::from_str::<Value>(&message.content) {
        Ok(value) => value,
        Err(_) if message.content.trim_start().starts_with(['{', '[']) => return Err(AiError::invalid("The MCP tool result is malformed JSON.")),
        Err(_) => return Ok(unchanged()),
    };
    let mut images = Vec::new();
    let mut bytes = 0usize;
    extract(&mut value, max_bytes, &mut bytes, &mut images, 0)?;
    if images.is_empty() {
        return Ok(unchanged());
    }
    let text = serde_json::to_string(&value).map_err(|_| AiError::invalid("Could not encode MCP image metadata."))?;
    Ok(ToolContent { text, images })
}

// These are the runtime's recovery and completed-delegation envelopes. Traverse only
// their recorded results; arbitrary application metadata is never interpreted as MCP.
fn extract(value: &mut Value, max_bytes: usize, bytes: &mut usize, images: &mut Vec<Image>, depth: usize) -> Result<(), AiError> {
    // A protocol nesting bound, independent of configurable byte budgets.
    if depth > 16 {
        return Err(AiError::invalid("MCP activity result nesting exceeds the supported limit."));
    }
    if let Some(content) = value.get_mut("content").and_then(Value::as_array_mut) {
        for block in content {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {}
                Some("image") => {
                    if block.get("mimeType").and_then(Value::as_str) != Some("image/png") {
                        return Err(AiError::invalid(
                            "This assistant supports inline MCP image/png images; the tool returned an unsupported image MIME type.",
                        ));
                    }
                    let data = block.get("data").and_then(Value::as_str).ok_or_else(|| AiError::invalid("The MCP image has no base64 data."))?;
                    let remaining = max_bytes.saturating_sub(*bytes);
                    if data.len() > remaining.div_ceil(3).saturating_mul(4) {
                        return Err(AiError::invalid("MCP image data exceeds the configured decoded-image byte limit."));
                    }
                    let decoded = base64::engine::general_purpose::STANDARD
                        .decode(data)
                        .map_err(|_| AiError::invalid("The MCP image is not valid standard base64."))?;
                    if decoded.len() > remaining {
                        return Err(AiError::invalid("MCP image data exceeds the configured decoded-image byte limit."));
                    }
                    if !decoded.starts_with(b"\x89PNG\r\n\x1a\n") {
                        return Err(AiError::invalid("The MCP image/png data has no PNG signature."));
                    }
                    *bytes = bytes.checked_add(decoded.len()).ok_or_else(|| AiError::invalid("MCP image byte count overflowed."))?;
                    images.push(Image { data_uri: format!("data:image/png;base64,{data}"), bytes: decoded.len() });
                    let object = block.as_object_mut().ok_or_else(|| AiError::invalid("The MCP image content is not an object."))?;
                    object.insert("data".into(), Value::String(format!("[{} decoded image bytes supplied separately]", decoded.len())));
                }
                Some(_) => {
                    return Err(AiError::invalid(
                        "The MCP tool returned unsupported content; only text and inline PNG images can be sent to this model adapter.",
                    ));
                }
                None => return Err(AiError::invalid("The MCP tool returned a content block without a type.")),
            }
        }
    }
    let envelope = value.get("status").and_then(Value::as_str) == Some("interrupted") || value.get("summary").is_some_and(Value::is_string);
    if envelope && let Some(activity) = value.get_mut("activity").and_then(Value::as_array_mut) {
        for record in activity {
            if let Some(result) = record.get_mut("result") {
                extract(result, max_bytes, bytes, images, depth + 1)?;
            }
        }
    }
    Ok(())
}

/// Clones only for text-budget accounting or textual transcript presentation.
/// The original messages retain exact raw MCP JSON and base64 data for persistence/continuation.
pub(crate) fn context_messages(messages: &[Message], config: &RuntimeConfig) -> Result<Vec<Message>, AiError> {
    let mut result = Vec::with_capacity(messages.len());
    let mut bytes = 0usize;
    for message in messages {
        let content = split(message, config.max_response_bytes.saturating_sub(bytes))?;
        for image in content.images {
            bytes = bytes.checked_add(image.bytes).ok_or_else(|| AiError::invalid("MCP image byte count overflowed."))?;
        }
        let mut message = message.clone();
        message.content = content.text;
        result.push(message);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    pub(crate) fn png() -> String {
        base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nsynthetic image")
    }
    #[test]
    fn image_metadata_error_flags_and_canonical_history_are_preserved() {
        let original = Message::tool(
            "shot",
            &json!({"content":[{"type":"image","mimeType":"image/png","data":png()},{"type":"text","text":"window metadata"}],"structuredContent":{"width":1},"isError":true,"_meta":{"test":"kept"}}),
        );
        let content = split(&original, 1024).unwrap();
        assert_eq!(content.images.len(), 1);
        assert_eq!(content.images[0].data_uri, format!("data:image/png;base64,{}", png()));
        let value: Value = serde_json::from_str(&content.text).unwrap();
        assert_eq!(value["isError"], true);
        assert_eq!(value["structuredContent"]["width"], 1);
        assert_eq!(value["_meta"]["test"], "kept");
        assert_eq!(value["content"][1]["text"], "window metadata");
        assert!(original.content.contains(&png()));
        assert!(!content.text.contains(&png()));
    }
    #[test]
    fn activity_envelopes_keep_metadata_and_enforce_shared_image_and_nesting_bounds() {
        let image = json!({"content":[{"type":"image","mimeType":"image/png","data":png()}],"isError":true,"_meta":{"kept":true}});
        let envelope = json!({"status":"interrupted","activity":[{"outcome":"completed","result":image},{"outcome":"completed","result":image}]});
        let message = Message::tool("delegate", &envelope);
        let split = super::split(&message, 1024).unwrap();
        assert_eq!(split.images.len(), 2);
        let sanitized: Value = serde_json::from_str(&split.text).unwrap();
        assert_eq!(sanitized["activity"][0]["result"]["_meta"]["kept"], true);
        assert_eq!(sanitized["activity"][0]["result"]["isError"], true);
        assert_eq!(serde_json::from_str::<Value>(&message.content).unwrap(), envelope);
        assert!(super::split(&message, split.images[0].bytes).is_err());
        let mut nested = image;
        for _ in 0..18 {
            nested = json!({"summary":"nested","activity":[{"result":nested}]});
        }
        assert!(super::split(&Message::tool("nested", &nested), 1024).err().unwrap().message.contains("nesting"));
    }
    #[test]
    fn non_image_text_is_byte_exact() {
        for content in ["plain tool text", r#" { "content": [{"type":"text", "text":"hello"}], "isError":false } "#] {
            let mut message = Message::text(MessageRole::Tool, content);
            message.tool_call_id = Some("call".into());
            assert_eq!(split(&message, 1).unwrap().text, content);
        }
    }
    #[test]
    fn malformed_unsupported_and_oversized_images_fail_explicitly() {
        for block in [
            json!({"type":"image","mimeType":"image/png","data":"%%%"}),
            json!({"type":"image","mimeType":"image/png","data":"aGVsbG8="}),
            json!({"type":"image","mimeType":"image/jpeg","data":png()}),
            json!({"type":"audio","data":"opaque"}),
            json!({"type":"image","mimeType":"image/png"}),
        ] {
            assert!(split(&Message::tool("bad", &json!({"content":[block]})), 1024).is_err());
        }
        assert!(split(&Message::text(MessageRole::Tool, "{broken MCP JSON"), 1024).is_err());
        let image = Message::tool("shot", &json!({"content":[{"type":"image","mimeType":"image/png","data":png()}]}));
        assert!(split(&image, 1).is_err());
        let config = RuntimeConfig { max_response_bytes: 30, ..Default::default() };
        assert!(context_messages(&[image.clone(), image], &config).is_err());
    }
}
