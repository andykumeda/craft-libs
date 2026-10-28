use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use craft_ai_core::assistant::{Message, MessageRole, RuntimeConfig};
use craft_ai_core::{AiError, Auth, Billing, Connection, ConnectionId, ErrorKind, Locality, ModelId, SecretRef, SecretString};
use craft_ai_native::secrets::SecretStore;
use craft_ai_native::{Cancellation, ChatProvider, HttpProvider, ModelRequest};

struct Key;
impl SecretStore for Key {
    fn get(&self, _: &SecretRef) -> Result<SecretString, AiError> {
        Ok(SecretString::new("synthetic-key"))
    }
    fn set(&self, _: &SecretRef, _: &SecretString) -> Result<(), AiError> {
        Ok(())
    }
    fn delete(&self, _: &SecretRef) -> Result<(), AiError> {
        Ok(())
    }
}
fn connection(endpoint: String) -> Connection {
    Connection {
        id: ConnectionId::new("test").unwrap(),
        label: "Test".into(),
        provider: "openai".into(),
        endpoint: Some(endpoint),
        locality: Locality::Local,
        auth: Auth::ApiKey { secret: SecretRef::new("synthetic").unwrap() },
        billing: Billing::Free,
        models: vec![],
        enabled: true,
        key_header: None,
    }
}
fn server(status: &str, body: &str, delay: Duration) -> (Connection, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let reply = format!("HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let worker = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut socket = loop {
            if let Ok((socket, _)) = listener.accept() {
                break socket;
            }
            assert!(Instant::now() < deadline, "test client never connected");
            std::thread::sleep(Duration::from_millis(5));
        };
        socket.set_nonblocking(false).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut received = Vec::new();
        let mut buffer = [0u8; 2048];
        loop {
            let n = socket.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            received.extend_from_slice(&buffer[..n]);
            assert!(received.len() < 1024 * 1024);
            if let Some(end) = received.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&received[..end]);
                let size = headers
                    .lines()
                    .find_map(|line| line.to_lowercase().strip_prefix("content-length:").and_then(|s| s.trim().parse::<usize>().ok()))
                    .unwrap_or(0);
                if received.len() >= end + 4 + size {
                    break;
                }
            }
        }
        std::thread::sleep(delay);
        let _ = socket.write_all(reply.as_bytes());
        String::from_utf8(received).unwrap()
    });
    (connection(format!("http://{address}/v1")), worker)
}

#[test]
fn streaming_request_carries_exact_model_and_secret_only_in_header() {
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"Done\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
    let (connection, worker) = server("200 OK", body, Duration::ZERO);
    let provider = HttpProvider::new(Arc::new(Key));
    let model = ModelId::new("chosen-model").unwrap();
    let messages = [Message::text(MessageRole::User, "synthetic prompt")];
    let config = RuntimeConfig::default();
    let reply = provider
        .complete(
            &ModelRequest { connection: &connection, model: &model, messages: &messages, tools: &[], config: &config },
            &Cancellation::new(),
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(reply.content, "Done");
    let request = worker.join().unwrap();
    assert!(request.starts_with("POST /v1/chat/completions "));
    assert!(request.to_lowercase().contains("authorization: bearer synthetic-key"));
    let body = request.split_once("\r\n\r\n").unwrap().1;
    assert!(!body.contains("synthetic-key"));
    assert!(body.contains("chosen-model"));
}

#[test]
fn authentication_error_body_never_leaks_credential() {
    let (connection, worker) = server("401 Unauthorized", "invalid synthetic-key", Duration::ZERO);
    let provider = HttpProvider::new(Arc::new(Key));
    let error = provider.models(&connection, &RuntimeConfig::default(), &Cancellation::new()).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Auth);
    assert!(!error.to_string().contains("synthetic-key"));
    worker.join().unwrap();
}

#[test]
fn stop_drops_an_inflight_request_without_waiting_for_provider_timeout() {
    let (connection, worker) = server("200 OK", "", Duration::from_millis(500));
    let provider = HttpProvider::new(Arc::new(Key));
    let cancel = Cancellation::new();
    let trigger = cancel.clone();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(75));
        trigger.cancel();
    });
    let error = provider.models(&connection, &RuntimeConfig::default(), &cancel).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Cancelled);
    stopper.join().unwrap();
    worker.join().unwrap();
}
