//! A bounded per-turn worker over the application's existing authorized tool host.

use std::collections::BTreeSet;
use std::sync::mpsc;
use std::time::Duration;

use craft_ai_core::AiError;
use craft_ai_core::assistant::{RuntimeConfig, Target, ToolDefinition};
use craft_ai_native::storage::SharedStore;
use craft_ai_native::{Cancellation, ToolHost};
use serde_json::Value;

use crate::protocol::{ToolRequest, ToolResult};

pub enum Event {
    Catalog { config: RuntimeConfig, tools: Vec<ToolDefinition>, initial_read: Option<(String, Value)> },
    Result { id: String, result: ToolResult },
    Failed(String),
}

pub struct Proxy<P> {
    pub pending: mpsc::Receiver<P>,
    pub events: mpsc::Receiver<Event>,
    sender: mpsc::SyncSender<(String, ToolRequest)>,
    cancel: Cancellation,
    target: Target,
    seen: BTreeSet<String>,
    max_calls: usize,
    max_arguments: usize,
}

impl<P: Send + 'static> Proxy<P> {
    pub fn spawn<H: ToolHost + 'static>(
        target: Target,
        wake: egui::Context,
        make: impl FnOnce(mpsc::Sender<P>, Cancellation, egui::Context, &RuntimeConfig) -> H + Send + 'static,
    ) -> Result<Self, String> {
        let (pending_tx, pending) = mpsc::channel();
        let (event_tx, events) = mpsc::sync_channel(1);
        let (sender, commands) = mpsc::sync_channel::<(String, ToolRequest)>(1);
        let cancel = Cancellation::new();
        let worker_cancel = cancel.clone();
        let worker_target = target.clone();
        std::thread::Builder::new()
            .name("assistant-app-tools".into())
            .spawn(move || {
                let run = || -> Result<(), String> {
                    let root = SharedStore::default_root().map_err(|e| e.to_string())?;
                    let config = SharedStore::new(root).config().map_err(|e| e.to_string())?;
                    let mut host = make(pending_tx, worker_cancel.clone(), wake.clone(), &config);
                    let tools = host.tools(&worker_target, &worker_cancel).map_err(|e| e.to_string())?;
                    let initial_read = host.initial_read();
                    event_tx.send(Event::Catalog { config, tools, initial_read }).map_err(|_| "The assistant panel closed".to_owned())?;
                    wake.request_repaint();
                    loop {
                        worker_cancel.check().map_err(|e| e.to_string())?;
                        match commands.recv_timeout(Duration::from_millis(50)) {
                            Ok((id, request)) => {
                                let result = host.call(&request.target, &request.call, &worker_cancel);
                                event_tx.send(Event::Result { id, result }).map_err(|_| "The assistant panel closed".to_owned())?;
                                wake.request_repaint();
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                        }
                    }
                };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run))
                    .unwrap_or_else(|_| Err("The app tool worker failed unexpectedly; completed actions will not be replayed.".into()));
                if let Err(error) = result {
                    let _ = event_tx.try_send(Event::Failed(error));
                    wake.request_repaint();
                }
            })
            .map_err(|e| format!("Could not start the assistant app tool worker ({e})"))?;
        Ok(Self { pending, events, sender, cancel, target, seen: BTreeSet::new(), max_calls: 0, max_arguments: 0 })
    }

    pub fn set_limits(&mut self, config: &RuntimeConfig) {
        self.max_calls = usize::try_from(config.max_rounds).unwrap_or(0).saturating_mul(config.max_tools).saturating_add(1);
        self.max_arguments = config.max_tool_arguments_bytes;
    }

    pub fn submit(&mut self, id: String, request: ToolRequest) -> Result<(), AiError> {
        self.cancel.check()?;
        if request.target.id != self.target.id || request.target.revision != self.target.revision {
            return Err(AiError::invalid("The assistant tool request targets a different document or turn."));
        }
        let arguments = serde_json::to_vec(&request.call.arguments).map_err(|_| AiError::invalid("Invalid assistant tool arguments."))?;
        if arguments.len() > self.max_arguments {
            return Err(AiError::invalid("The assistant tool arguments exceed the configured limit."));
        }
        if id.is_empty()
            || id.len() > 128
            || request.call.id.is_empty()
            || request.call.id.len() > 128
            || self.seen.len() / 2 >= self.max_calls
            || !self.seen.insert(format!("pipe:{id}"))
            || !self.seen.insert(format!("call:{}", request.call.id))
        {
            return Err(AiError::invalid("The assistant tool request is duplicate, invalid or exceeds the turn limit; it will not be replayed."));
        }
        self.sender
            .try_send((id, request))
            .map_err(|_| AiError::invalid("An assistant tool is already running or the turn ended; the request was not queued."))
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl<P> Drop for Proxy<P> {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_ai_core::assistant::ToolCall;
    use serde_json::json;

    fn proxy() -> (Proxy<()>, mpsc::Receiver<(String, ToolRequest)>) {
        let (_, pending) = mpsc::channel();
        let (_, events) = mpsc::channel();
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut proxy = Proxy {
            pending,
            events,
            sender,
            cancel: Cancellation::new(),
            target: Target { id: "document-one".into(), revision: 7 },
            seen: BTreeSet::new(),
            max_calls: 0,
            max_arguments: 0,
        };
        proxy.set_limits(&RuntimeConfig::default());
        (proxy, receiver)
    }
    fn request() -> ToolRequest {
        ToolRequest {
            target: Target { id: "document-one".into(), revision: 7 },
            call: ToolCall { id: "tool-one".into(), name: "read".into(), arguments: json!({}) },
        }
    }
    #[test]
    fn wrong_targets_duplicate_tool_ids_and_canceled_turns_never_reach_the_worker() {
        let (mut proxy, receiver) = proxy();
        let mut wrong = request();
        wrong.target.revision = 8;
        assert!(proxy.submit("one".into(), wrong).is_err());
        assert!(receiver.try_recv().is_err());
        proxy.submit("one".into(), request()).unwrap();
        receiver.recv().unwrap();
        assert!(proxy.submit("one".into(), request()).is_err());
        assert!(proxy.submit("different-envelope".into(), request()).is_err());
        assert!(receiver.try_recv().is_err());
        proxy.cancel();
        let mut next = request();
        next.call.id = "next-tool".into();
        assert!(proxy.submit("next".into(), next).is_err());
        assert!(receiver.try_recv().is_err());
    }
    #[test]
    fn a_full_worker_queue_and_oversized_arguments_are_refused_without_replay() {
        let (mut proxy, receiver) = proxy();
        proxy.submit("one".into(), request()).unwrap();
        let mut next = request();
        next.call.id = "next-tool".into();
        assert!(proxy.submit("next".into(), next.clone()).is_err());
        receiver.recv().unwrap();
        assert!(proxy.submit("next".into(), next).is_err());
        proxy.max_arguments = 1;
        let mut large = request();
        large.call.id = "large".into();
        assert!(proxy.submit("large".into(), large).is_err());
        assert!(receiver.try_recv().is_err());
    }
}
