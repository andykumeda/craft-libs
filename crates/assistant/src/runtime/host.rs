//! A bounded, cancellable proxy to the application's authorized document tool host.
use craft_ai_core::{
    AiError, ErrorKind,
    assistant::{Target, ToolCall, ToolDefinition},
};
use craft_ai_native::{Cancellation, ToolHost};
use serde_json::Value;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
pub(crate) struct Pending {
    pub call: ToolCall,
    pub target: Target,
    pub reply: mpsc::Sender<Result<Value, AiError>>,
    pub cancel: Cancellation,
    pub expires: Instant,
}
pub(super) struct McpHost {
    sender: mpsc::Sender<Pending>,
    cancel: Cancellation,
    wake: egui::Context,
    timeout: Duration,
    limit: usize,
    tools: Vec<ToolDefinition>,
    initial_read: Option<(String, Value)>,
}
impl McpHost {
    pub fn new(
        sender: mpsc::Sender<Pending>,
        cancel: Cancellation,
        wake: egui::Context,
        timeout: Duration,
        limit: usize,
        tools: Vec<ToolDefinition>,
        initial_read: Option<(String, Value)>,
    ) -> Self {
        Self { sender, cancel, wake, timeout, limit, tools, initial_read }
    }
}
impl ToolHost for McpHost {
    fn initial_read(&self) -> Option<(String, Value)> {
        self.initial_read.clone()
    }
    fn tools(&mut self, _: &Target, cancel: &Cancellation) -> Result<Vec<ToolDefinition>, AiError> {
        cancel.check()?;
        Ok(self.tools.clone())
    }
    fn call(&mut self, target: &Target, call: &ToolCall, cancel: &Cancellation) -> Result<Value, AiError> {
        cancel.check()?;
        self.cancel.check()?;
        if !self.tools.iter().any(|tool| tool.name == call.name) {
            return Err(AiError::invalid("tool is not in this app's catalog"));
        }
        if serde_json::to_vec(call).map_err(|_| AiError::invalid("invalid tool request"))?.len() > self.limit {
            return Err(AiError::invalid("tool request exceeds context budget"));
        }
        let expires = Instant::now().checked_add(self.timeout).ok_or_else(|| AiError::invalid("invalid assistant timeout"))?;
        let (reply, receive) = mpsc::channel();
        self.sender
            .send(Pending { call: call.clone(), target: target.clone(), reply, cancel: self.cancel.clone(), expires })
            .map_err(|_| AiError::cancelled())?;
        self.wake.request_repaint();
        loop {
            cancel.check()?;
            self.cancel.check()?;
            match receive.recv_timeout(Duration::from_millis(25)) {
                Ok(value) => return value,
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < expires => {}
                _ => {
                    self.cancel.cancel();
                    return Err(AiError::new(ErrorKind::Network, "App response unavailable. Outcome uncertain; no action will be replayed."));
                }
            }
        }
    }
}
