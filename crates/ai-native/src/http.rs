//! OpenAI-compatible streaming text and tool calls. No redirects, fallback, or retries.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use craft_ai_core::assistant::{Message, MessageRole, RuntimeConfig, ToolCall};
use craft_ai_core::{AiError, Auth, Connection, ErrorKind, ModelId, ModelInfo, SecretString};
use serde_json::{Value, json};

use crate::runtime::{Cancellation, ChatProvider, ModelRequest};
use crate::secrets::SecretStore;

pub struct HttpProvider {
    secrets: Arc<dyn SecretStore>,
}
impl HttpProvider {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self { secrets }
    }
}

impl ChatProvider for HttpProvider {
    fn complete(&self, request: &ModelRequest<'_>, cancel: &Cancellation, emit: &mut dyn FnMut(&str)) -> Result<Message, AiError> {
        request.config.validate()?;
        let key = self.key(request.connection)?;
        let result = run_async(async {
            let client = client(request.config)?;
            let url = endpoint(request.connection, "chat/completions")?;
            let builder = authorize(client.post(url), request.connection, key.as_ref())?;
            let response = interrupt(builder.json(&payload(request)).send(), cancel).await?.map_err(network)?;
            let response = status(response, request.connection, Some(request.model), request.config, cancel).await?;
            let mut parser = StreamParser::new(request.config);
            let mut redactor = StreamRedactor::new(key.as_ref().map(SecretString::expose));
            let mut response = response;
            loop {
                let chunk = interrupt(response.chunk(), cancel).await?.map_err(network)?;
                let Some(bytes) = chunk else { break };
                parser.push(&bytes, &mut |text| redactor.push(text, emit))?;
            }
            let mut answer = parser.finish()?;
            if let Some(key) = &key {
                answer.content = answer.content.replace(key.expose(), "[REDACTED]");
                for call in &mut answer.tool_calls {
                    call.id = call.id.replace(key.expose(), "[REDACTED]");
                    call.name = call.name.replace(key.expose(), "[REDACTED]");
                    redact_value(&mut call.arguments, key.expose());
                }
            }
            redactor.finish(emit);
            cancel.check()?;
            Ok(answer)
        });
        redact(result, key.as_ref())
    }

    fn models(&self, connection: &Connection, config: &RuntimeConfig, cancel: &Cancellation) -> Result<Vec<ModelInfo>, AiError> {
        config.validate()?;
        let key = self.key(connection)?;
        let result = run_async(async {
            let client = client(config)?;
            let request = authorize(client.get(endpoint(connection, "models")?), connection, key.as_ref())?;
            let response = interrupt(request.send(), cancel).await?.map_err(network)?;
            let mut response = status(response, connection, None, config, cancel).await?;
            let mut bytes = Vec::new();
            while let Some(chunk) = interrupt(response.chunk(), cancel).await?.map_err(network)? {
                if bytes.len().saturating_add(chunk.len()) > config.max_response_bytes {
                    return Err(AiError::invalid("model catalog exceeds configured response limit"));
                }
                bytes.extend_from_slice(&chunk);
            }
            let mut value: Value =
                serde_json::from_slice(&bytes).map_err(|_| AiError::new(ErrorKind::Provider, "provider returned an invalid model catalog"))?;
            if let Some(key) = &key {
                redact_value(&mut value, key.expose());
            }
            let rows = value
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::new(ErrorKind::Provider, "provider model catalog has no data array"))?;
            if rows.len() > 10_000 {
                return Err(AiError::invalid("provider model catalog contains too many entries"));
            }
            let mut models = Vec::new();
            for row in rows {
                let Some(id) = row.get("id").and_then(Value::as_str).and_then(|s| ModelId::new(s).ok()) else { continue };
                if models.iter().any(|m: &ModelInfo| m.id == id) {
                    continue;
                }
                let mut model = craft_ai_core::assistant::tool_model(id)?;
                if let Some(name) = row.get("name").and_then(Value::as_str).filter(|s| s.len() <= 512) {
                    model.label = name.into();
                }
                let advertised = row
                    .get("supported_parameters")
                    .and_then(Value::as_array)
                    .is_some_and(|parameters| parameters.iter().any(|p| p.as_str() == Some("tools")));
                for capability in &mut model.capabilities {
                    capability.features.tools = advertised;
                }
                models.push(model);
            }
            Ok(models)
        });
        redact(result, key.as_ref())
    }
}

impl HttpProvider {
    fn key(&self, connection: &Connection) -> Result<Option<SecretString>, AiError> {
        connection.validate().map_err(|e| AiError::invalid(e.to_string()))?;
        match &connection.auth {
            Auth::None => Ok(None),
            Auth::ApiKey { secret } => {
                let key = self.secrets.get(secret)?;
                if key.is_empty() || key.expose().len() > craft_ai_core::draft::MAX_KEY_BYTES {
                    return Err(AiError::new(ErrorKind::Auth, "the configured API credential is empty or exceeds the supported size"));
                }
                Ok(Some(key))
            }
            Auth::Subscription { .. } => Err(AiError::unsupported("subscription connections require their selected subscription adapter")),
        }
    }
}

fn redact<T>(result: Result<T, AiError>, key: Option<&SecretString>) -> Result<T, AiError> {
    result.map_err(|e| match key {
        Some(key) => AiError::redacted(e.kind, &e.message, &[key]),
        None => e,
    })
}

// A provider may echo a credential in a response. Retain the suffix across chunks so
// fragment boundaries cannot expose a key in the UI before the final response is scrubbed.
struct StreamRedactor<'a> {
    secret: Option<&'a str>,
    pending: String,
}
impl<'a> StreamRedactor<'a> {
    fn new(secret: Option<&'a str>) -> Self {
        Self { secret: secret.filter(|key| !key.is_empty()), pending: String::new() }
    }
    fn push(&mut self, text: &str, emit: &mut dyn FnMut(&str)) {
        let Some(secret) = self.secret else {
            emit(text);
            return;
        };
        self.pending.push_str(text);
        while let Some(at) = self.pending.find(secret) {
            emit(&self.pending[..at]);
            emit("[REDACTED]");
            self.pending.drain(..at + secret.len());
        }
        let mut end = self.pending.len().saturating_sub(secret.len().saturating_sub(1));
        while !self.pending.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        if end > 0 {
            emit(&self.pending[..end]);
            self.pending.drain(..end);
        }
    }
    fn finish(self, emit: &mut dyn FnMut(&str)) {
        if !self.pending.is_empty() {
            emit(&self.pending);
        }
    }
}

fn redact_value(value: &mut Value, secret: &str) {
    if secret.is_empty() {
        return;
    }
    match value {
        Value::String(text) => *text = text.replace(secret, "[REDACTED]"),
        Value::Array(items) => items.iter_mut().for_each(|item| redact_value(item, secret)),
        Value::Object(fields) => {
            let redacted = std::mem::take(fields)
                .into_iter()
                .map(|(name, mut value)| {
                    redact_value(&mut value, secret);
                    (name.replace(secret, "[REDACTED]"), value)
                })
                .collect();
            *fields = redacted;
        }
        _ => {}
    }
}

fn endpoint(connection: &Connection, path: &str) -> Result<url::Url, AiError> {
    connection.validate().map_err(|e| AiError::invalid(e.to_string()))?;
    let base = connection.endpoint.as_deref().ok_or_else(|| AiError::invalid("this connection has no API endpoint"))?;
    let mut url = url::Url::parse(base).map_err(|_| AiError::invalid("API endpoint is invalid"))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AiError::invalid("API endpoint must be HTTP(S), without credentials, query, or fragment"));
    }
    let next = format!("{}/{}", url.path().trim_end_matches('/'), path);
    url.set_path(&next);
    Ok(url)
}

fn client(config: &RuntimeConfig) -> Result<reqwest::Client, AiError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(config.connect_timeout_secs))
        .timeout(Duration::from_secs(config.request_timeout_secs))
        .build()
        .map_err(network)
}

fn authorize(builder: reqwest::RequestBuilder, connection: &Connection, key: Option<&SecretString>) -> Result<reqwest::RequestBuilder, AiError> {
    match key {
        Some(key) => match &connection.key_header {
            Some(name) => {
                craft_ai_core::connection::check_key_header(name).map_err(AiError::invalid)?;
                Ok(builder.header(name, key.expose()))
            }
            None => Ok(builder.bearer_auth(key.expose())),
        },
        None => Ok(builder),
    }
}

async fn status(
    mut response: reqwest::Response,
    connection: &Connection,
    model: Option<&ModelId>,
    config: &RuntimeConfig,
    cancel: &Cancellation,
) -> Result<reqwest::Response, AiError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let code = response.status().as_u16();
    let kind = match code {
        401 | 403 => ErrorKind::Auth,
        429 => ErrorKind::RateLimited,
        _ => ErrorKind::Provider,
    };
    let guidance = match code {
        400 | 422 => {
            "The provider rejected the request. Check that the selected model supports streaming and tool calls, and that the conversation fits its context limit."
        }
        401 => "The provider rejected this connection's API key. Check the saved credential in Assistant settings.",
        403 => "This account cannot access the requested resource. Check the account's model permissions and access policy.",
        404 => "The API route or model was not found. Check the API base URL and selected model ID in Assistant settings.",
        413 => "The request is too large for this provider. Start a new chat or reduce the document content in the request.",
        429 => "The provider's rate or usage limit was reached. Check the account's quota and wait before trying again.",
        500..=599 => "The provider could not complete the request. Check its service status before trying again.",
        _ => "Check this connection's API base URL and selected model in Assistant settings.",
    };
    let target = model.map_or_else(|| connection.label.clone(), |model| format!("{} / {}", connection.label, model.as_str()));
    if code == 400 && connection.provider == "ollama" {
        let mut bytes = Vec::new();
        while let Ok(Some(chunk)) = interrupt(response.chunk(), cancel).await? {
            if bytes.len().saturating_add(chunk.len()) > config.max_response_bytes {
                bytes.clear();
                break;
            }
            bytes.extend_from_slice(&chunk);
        }
        if let Some((needed, available)) = ollama_context_overflow(&bytes) {
            return Err(AiError::new(
                ErrorKind::InvalidRequest,
                format!(
                    "{target}: context too small ({needed} tokens needed; {available} available). Increase this model's context size in Ollama. Document and tool results count even in a new chat. No automatic retry was attempted."
                ),
            ));
        }
    }
    // Provider bodies may echo private prompts or credentials; display only our own guidance.
    Err(AiError::new(kind, format!("{target}: HTTP {code}. {guidance} No automatic retry was attempted.")))
}

// Extract only counts from Ollama's known rejection; never forward provider-supplied text.
fn ollama_context_overflow(bytes: &[u8]) -> Option<(u64, u64)> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let message = value.get("error")?.get("message")?.as_str()?;
    let (needed, available) = message.strip_prefix("request (")?.split_once(" tokens) exceeds the available context size (")?;
    let available = available.strip_suffix(" tokens), try increasing it")?;
    let needed = needed.parse::<u64>().ok()?;
    let available = available.parse::<u64>().ok()?;
    (needed > available && available > 0).then_some((needed, available))
}

fn network(error: reqwest::Error) -> AiError {
    let message = if error.is_timeout() {
        "provider request timed out; no automatic retry was attempted"
    } else if error.is_connect() {
        "could not connect to the configured provider"
    } else {
        "provider request failed; no automatic retry was attempted"
    };
    AiError::new(ErrorKind::Network, message)
}

fn run_async<T>(future: impl std::future::Future<Output = Result<T, AiError>>) -> Result<T, AiError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| AiError::new(ErrorKind::Internal, "could not start the provider runtime"))?
        .block_on(future)
}

async fn interrupt<T>(future: impl std::future::Future<Output = T>, cancel: &Cancellation) -> Result<T, AiError> {
    cancel.check()?;
    tokio::pin!(future);
    loop {
        tokio::select! {
            result = &mut future => return Ok(result),
            _ = tokio::time::sleep(Duration::from_millis(25)) => cancel.check()?,
        }
    }
}

fn payload(request: &ModelRequest<'_>) -> Value {
    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(|message| {
            let role = match message.role {
                MessageRole::System => "system",
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::Tool => "tool",
            };
            let mut value = json!({"role":role,"content":message.content});
            if !message.tool_calls.is_empty() {
                value["tool_calls"] = Value::Array(
                    message
                        .tool_calls
                        .iter()
                        .map(|call| json!({"id":call.id,"type":"function","function":{"name":call.name,"arguments":call.arguments.to_string()}}))
                        .collect(),
                );
            }
            if let Some(id) = &message.tool_call_id {
                value["tool_call_id"] = json!(id);
            }
            value
        })
        .collect();
    let mut body = json!({"model":request.model.as_str(),"messages":messages,"stream":true});
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(
            request
                .tools
                .iter()
                .map(|tool| json!({"type":"function","function":{"name":tool.name,"description":tool.description,"parameters":tool.input_schema}}))
                .collect(),
        );
    }
    body
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

struct StreamParser<'a> {
    config: &'a RuntimeConfig,
    buffer: Vec<u8>,
    data: String,
    bytes: usize,
    text: String,
    calls: BTreeMap<u64, PartialCall>,
    finished: bool,
    done: bool,
}

impl<'a> StreamParser<'a> {
    fn new(config: &'a RuntimeConfig) -> Self {
        Self { config, buffer: Vec::new(), data: String::new(), bytes: 0, text: String::new(), calls: BTreeMap::new(), finished: false, done: false }
    }

    fn push(&mut self, bytes: &[u8], emit: &mut dyn FnMut(&str)) -> Result<(), AiError> {
        self.bytes = self.bytes.saturating_add(bytes.len());
        if self.bytes > self.config.max_response_bytes {
            return Err(AiError::invalid("provider stream exceeds configured response limit"));
        }
        self.buffer.extend_from_slice(bytes);
        while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let text = std::str::from_utf8(&line)
                .map_err(|_| AiError::new(ErrorKind::Provider, "provider stream contains invalid UTF-8"))?
                .trim_end_matches(['\r', '\n']);
            if text.is_empty() {
                self.event(emit)?;
            } else if let Some(data) = text.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(data.strip_prefix(' ').unwrap_or(data));
            }
        }
        Ok(())
    }

    fn event(&mut self, emit: &mut dyn FnMut(&str)) -> Result<(), AiError> {
        let data = std::mem::take(&mut self.data);
        if data.is_empty() {
            return Ok(());
        }
        if data == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        if self.done {
            return Err(AiError::new(ErrorKind::Provider, "provider sent content after its completed stream"));
        }
        let value: Value = serde_json::from_str(&data).map_err(|_| AiError::new(ErrorKind::Provider, "provider returned malformed stream data"))?;
        if value.get("error").is_some() {
            return Err(AiError::new(ErrorKind::Provider, "provider reported a streaming error"));
        }
        let Some(choice) = value.get("choices").and_then(Value::as_array).and_then(|v| v.first()) else {
            return Ok(());
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            if !matches!(reason, "stop" | "tool_calls") {
                return Err(AiError::new(ErrorKind::Provider, "provider stopped before a complete answer; no tool was executed"));
            }
            self.finished = true;
        }
        let delta = &choice["delta"];
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            self.text.push_str(text);
            emit(text);
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                self.call_delta(call)?;
            }
        }
        Ok(())
    }

    fn call_delta(&mut self, call: &Value) -> Result<(), AiError> {
        let index = call.get("index").and_then(Value::as_u64).ok_or_else(|| AiError::new(ErrorKind::Provider, "tool fragment has no index"))?;
        if !self.calls.contains_key(&index) && self.calls.len() >= self.config.max_tools {
            return Err(AiError::invalid("provider requested too many tools"));
        }
        let partial = self.calls.entry(index).or_default();
        if let Some(id) = call.get("id").and_then(Value::as_str) {
            partial.id.push_str(id);
        }
        if let Some(function) = call.get("function") {
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                partial.name.push_str(name);
            }
            if let Some(args) = function.get("arguments").and_then(Value::as_str) {
                partial.arguments.push_str(args);
            }
        }
        if partial.id.len() > 256 || partial.name.len() > 256 || partial.arguments.len() > self.config.max_tool_arguments_bytes {
            return Err(AiError::invalid("provider tool call exceeds configured limits"));
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Message, AiError> {
        if !self.buffer.is_empty() || !self.data.is_empty() || !(self.done || self.finished) {
            return Err(AiError::new(ErrorKind::Network, "provider stream ended before completion; no tool was executed"));
        }
        let mut calls = Vec::new();
        for call in std::mem::take(&mut self.calls).into_values() {
            if call.id.is_empty() || call.name.is_empty() {
                return Err(AiError::new(ErrorKind::Provider, "provider returned an incomplete tool call"));
            }
            let arguments =
                serde_json::from_str(&call.arguments).map_err(|_| AiError::new(ErrorKind::Provider, "provider returned malformed tool arguments"))?;
            calls.push(ToolCall { id: call.id, name: call.name, arguments });
        }
        Ok(Message { role: MessageRole::Assistant, content: self.text, tool_calls: calls, tool_call_id: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragmented_utf8_and_tool_arguments_survive() {
        let config = RuntimeConfig::default();
        let mut parser = StreamParser::new(&config);
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"café\",\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"edit\",\"arguments\":\"{\\\"text\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"hi\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut text = String::new();
        for byte in stream.as_bytes() {
            parser.push(&[*byte], &mut |s| text.push_str(s)).unwrap();
        }
        let reply = parser.finish().unwrap();
        assert_eq!(text, "café");
        assert_eq!(reply.tool_calls.first().unwrap().arguments, json!({"text":"hi"}));
    }

    #[test]
    fn truncated_or_oversized_stream_cannot_execute_tools() {
        let config = RuntimeConfig { max_response_bytes: 1024, ..Default::default() };
        let mut parser = StreamParser::new(&config);
        parser.push(b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n", &mut |_| {}).unwrap();
        assert!(parser.finish().is_err());
        let mut parser = StreamParser::new(&config);
        assert!(parser.push(&vec![b'x'; 1025], &mut |_| {}).is_err());
    }

    #[test]
    fn credentials_are_redacted_across_every_stream_boundary() {
        let secret = "synthetic-secret-123";
        for split in 0..=secret.len() {
            let mut redactor = StreamRedactor::new(Some(secret));
            let mut output = String::new();
            redactor.push("café ", &mut |text| output.push_str(text));
            redactor.push(&secret[..split], &mut |text| output.push_str(text));
            redactor.push(&secret[split..], &mut |text| output.push_str(text));
            redactor.push(" suffix", &mut |text| output.push_str(text));
            redactor.finish(&mut |text| output.push_str(text));
            assert_eq!(output, "café [REDACTED] suffix");
        }
        let mut arguments = json!({"nested":[{"synthetic-secret-123":"prefix synthetic-secret-123 suffix"}]});
        redact_value(&mut arguments, secret);
        assert!(!arguments.to_string().contains(secret));
    }
}
