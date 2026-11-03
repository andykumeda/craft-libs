use crate::wire::{HelperMessage, HostMessage, Limits, read_message, write_message};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
};

/// Private pipes only. Bounded queues apply backpressure without blocking the app's UI thread.
pub struct Client {
    child: Child,
    tx: Option<SyncSender<HostMessage>>,
    rx: Receiver<Result<HelperMessage, String>>,
    error: Option<String>,
}
impl Client {
    pub fn spawn(executable: &Path, limits: Limits) -> Result<Self, String> {
        limits.validate()?;
        let mut child = Command::new(executable)
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start assistant: {e}"))?;
        let (Some(mut stdin), Some(mut stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("could not create private assistant pipes".into());
        };
        let (tx, commands) = mpsc::sync_channel::<HostMessage>(8);
        let (events, rx) = mpsc::sync_channel(8);
        let errors = events.clone();
        let max = limits.max_message_bytes;
        if let Err(error) = std::thread::Builder::new().name("assistant-pipe-write".into()).spawn(move || {
            while let Ok(message) = commands.recv() {
                if let Err(error) = write_message(&mut stdin, &message, max) {
                    let _ = errors.send(Err(error));
                    break;
                }
            }
        }) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.to_string());
        }
        if let Err(error) = std::thread::Builder::new().name("assistant-pipe-read".into()).spawn(move || {
            loop {
                match read_message::<HelperMessage>(&mut stdout, max) {
                    Ok(Some(message)) => {
                        if events.send(Ok(message)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {
                        let _ = events.send(Err("assistant process closed its connection".into()));
                        break;
                    }
                    Err(error) => {
                        let _ = events.send(Err(error));
                        break;
                    }
                }
            }
        }) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.to_string());
        }
        Ok(Self { child, tx: Some(tx), rx, error: None })
    }
    /// A full queue refuses the command; it is never silently dropped or automatically replayed.
    pub fn send(&mut self, message: HostMessage) -> Result<(), String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        match self.tx.as_ref().ok_or("assistant connection is closed")?.try_send(message) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err("assistant is busy receiving requests; try again when idle".into()),
            Err(TrySendError::Disconnected(_)) => {
                let error = "assistant process is unavailable".to_string();
                self.error = Some(error.clone());
                Err(error)
            }
        }
    }
    pub fn poll(&mut self) -> Vec<HelperMessage> {
        let mut events = Vec::new();
        for _ in 0..8 {
            match self.rx.try_recv() {
                Ok(Ok(event)) => events.push(event),
                Ok(Err(error)) => {
                    self.error = Some(error);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.error.is_none() {
                        self.error = Some("assistant process stopped".into())
                    }
                    break;
                }
            }
        }
        events
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.tx.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
