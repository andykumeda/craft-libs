//! Native assistant services. Applications own documents and authorize every MCP operation.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod codex;
pub mod http;
pub mod runtime;
pub mod secrets;
pub mod storage;

pub use http::HttpProvider;
pub use runtime::{Cancellation, ChatProvider, ModelChoice, ModelRequest, ToolHost, TurnEvent, TurnRequest, resolve_choices, run_turn, verify_model};
