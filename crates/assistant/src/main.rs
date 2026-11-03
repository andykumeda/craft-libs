//! Opt-in private-pipe assistant executable; never registers MCP or opens a listener.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
fn main() -> std::process::ExitCode {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--stdio"] {
        eprintln!("Launch craft-assistant through a configured Craft app using --stdio.");
        return std::process::ExitCode::FAILURE;
    }
    let result = std::panic::catch_unwind(|| craft_assistant::server::serve(std::io::stdin(), std::io::stdout()));
    match result {
        Ok(Ok(())) => std::process::ExitCode::SUCCESS,
        _ => {
            eprintln!("The assistant stopped. Reopen the panel; inspect completed edits before continuing.");
            std::process::ExitCode::FAILURE
        }
    }
}
