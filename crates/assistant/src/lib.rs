//! Independently installed assistant services and embedding.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(feature = "runtime")]
pub mod runtime;

pub mod protocol;
#[cfg(feature = "runtime")]
pub mod server;

pub mod config;
pub mod updates;

#[cfg(not(target_arch = "wasm32"))]
pub mod controller;

#[cfg(not(target_arch = "wasm32"))]
pub mod host_proxy;
