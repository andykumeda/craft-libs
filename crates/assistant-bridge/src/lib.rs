//! A bounded, versioned private-pipe interface for independently installed assistant panels.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod client;
mod input;
mod render;
pub mod wire;
pub use client::Client;
pub use input::capture_input;
pub use render::{EmbeddedPanel, PanelRenderer};
pub use wire::{HelperMessage, HostMessage, Limits, PROTOCOL, PanelFrame, PanelInput};

#[cfg(test)]
mod tests;
