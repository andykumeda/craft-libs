//! Private-pipe executable service. The main loop owns UI state; worker threads own inference.
use crate::{
    protocol::{Event, Initialize, RestartState, StartTurn, Status, ToolRequest, ToolResult},
    runtime::Session,
};
use craft_ai_core::{AiError, AppId};
use craft_assistant_bridge::wire::{read_message, write_message};
use craft_assistant_bridge::{HelperMessage, HostMessage, PROTOCOL, PanelRenderer};
use craft_ui::assistant::Action;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    sync::mpsc,
    time::Instant,
};

fn send(output: &mut impl Write, value: HelperMessage, max: usize) -> Result<(), String> {
    write_message(output, &value, max)
}
fn event(output: &mut impl Write, value: Event, max: usize) -> Result<(), String> {
    send(output, HelperMessage::Event(serde_json::to_value(value).map_err(|_| "assistant event serialization failed")?), max)
}
/// Bootstrap framing is bounded before configuration is accepted; this is a protocol constant.
const BOOTSTRAP_BYTES: usize = 1024 * 1024;

pub fn serve(mut input: impl Read + Send + 'static, mut output: impl Write) -> Result<(), String> {
    let Some(HostMessage::Initialize { protocol, app_id, config }) = read_message(&mut input, BOOTSTRAP_BYTES)? else {
        return Err("initialize the assistant before sending commands".into());
    };
    if protocol != PROTOCOL {
        return Err("assistant bridge is incompatible; update the app or assistant".into());
    }
    let app = AppId::new(app_id).map_err(|_| "invalid assistant app identity")?;
    let mut config: Initialize = serde_json::from_value(config).map_err(|_| "invalid assistant initialization")?;
    config.limits.validate()?;
    let limits = config.limits;
    let max = limits.max_message_bytes;
    let mut renderer = PanelRenderer::new(limits.clone())?;
    let context = renderer.context().clone();
    let mut session = Session::new(app, context);
    session.translations = config.translations.clone();
    let (sender, receive) = mpsc::sync_channel(8);
    std::thread::Builder::new()
        .name("assistant-input".into())
        .spawn(move || {
            loop {
                match read_message::<HostMessage>(&mut input, max) {
                    Ok(Some(message)) => {
                        if sender.send(Ok(message)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
        })
        .map_err(|e| format!("could not start assistant input: {e}"))?;
    send(
        &mut output,
        HelperMessage::Ready {
            protocol: PROTOCOL,
            version: env!("CARGO_PKG_VERSION").into(),
            capabilities: vec!["render-v1".into(), "tools-v1".into(), "shared-settings-v1".into()],
        },
        max,
    )?;
    let mut replies = std::collections::BTreeMap::new();
    let mut next_call = 0u64;
    let mut last_reload = Instant::now();
    let mut restore_chat = config.restore_chat.take();
    let mut pending_turn: Option<String> = None;
    loop {
        let message = match receive.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(message) => Some(message?),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        session.poll();
        let missing_status = std::mem::take(session.missing_translations.get_mut());
        if !missing_status.is_empty() {
            event(&mut output, Event::Translate { keys: missing_status.into_iter().collect() }, max)?;
        }
        if !session.state().settings.busy
            && let Some(id) = restore_chat.take()
            && let Err(error) = session.action(Action::OpenChat(id))
        {
            session.state().error = Some(error);
        }
        replies.retain(|_, pending: &mut crate::runtime::host::Pending| {
            if pending.cancel.is_cancelled() || Instant::now() >= pending.expires {
                let _ = pending.reply.send(Err(AiError::cancelled()));
                false
            } else {
                true
            }
        });
        if let Some(message) = message {
            match message {
                HostMessage::Initialize { .. } => return Err("assistant is already initialized".into()),
                HostMessage::Shutdown => break,
                HostMessage::ToolResult { id, result } => {
                    if let Some(pending) = replies.remove(&id) {
                        let pending: crate::runtime::host::Pending = pending;
                        let result: ToolResult =
                            serde_json::from_value(result).unwrap_or_else(|_| Err(AiError::invalid("invalid app tool response")));
                        let _ = pending.reply.send(result);
                    } else if id.parse::<u64>().map_or(true, |id| id == 0 || id > next_call) {
                        return Err("unsolicited app tool response".into());
                    }
                }
                HostMessage::Command { id, name, args } => {
                    let result = if name == "restart_state" {
                        let state = session.state();
                        let safe = pending_turn.is_none()
                            && !state.busy
                            && !state.settings.busy
                            && state.draft.is_empty()
                            && state.file_path.is_empty()
                            && state.authorized_files.is_empty()
                            && state.error.is_none()
                            && !state.settings_open
                            && !state.settings.has_unsaved_edits();
                        let result = serde_json::to_value(RestartState { safe, chat_id: state.active_chat.clone() })
                            .map_err(|_| "invalid restart state".to_owned());
                        event(&mut output, Event::Result { id, result }, max)?;
                        continue;
                    } else if name == "translations" {
                        match serde_json::from_value::<std::collections::BTreeMap<String, String>>(args) {
                            Ok(entries) if entries.len() <= 1024 && entries.iter().all(|(k, v)| k.len() <= 4096 && v.len() <= 16384) => {
                                session.translations.extend(entries.clone());
                                let state = session.state();
                                for value in [&mut state.status, &mut state.settings.status] {
                                    if let Some(text) = value
                                        && let Some(translated) = entries.get(text)
                                    {
                                        *text = translated.clone();
                                    }
                                }
                                config.translations.extend(entries);
                                Ok(())
                            }
                            _ => Err("invalid assistant translations".into()),
                        }
                    } else if name == "reset_translations" {
                        config.translations.clear();
                        session.translations.clear();
                        Ok(())
                    } else if name == "reject_turn" {
                        if let Some(prompt) = pending_turn.take()
                            && session.state().draft.is_empty()
                        {
                            session.state().draft = prompt;
                        }
                        session.state().error = Some(args.as_str().unwrap_or("The app refused this assistant request.").to_owned());
                        Ok(())
                    } else {
                        let handoff = if name == "start_turn" || name == "stop" { pending_turn.take() } else { None };
                        let result = command(&mut session, &name, args);
                        if name == "start_turn"
                            && result.is_err()
                            && let Some(prompt) = handoff
                            && session.state().draft.is_empty()
                        {
                            session.state().draft = prompt;
                        }
                        result
                    };
                    event(&mut output, Event::Result { id, result: result.map(|()| json!({"accepted":true})) }, max)?;
                }
                HostMessage::Frame { input } => {
                    if pending_turn.is_none()
                        && !session.state().busy
                        && !session.state().settings.busy
                        && !session.state().settings_open
                        && !session.state().settings.has_unsaved_edits()
                        && last_reload.elapsed().as_secs() >= 1
                    {
                        if let Err(error) = session.refresh_settings() {
                            session.state().error = Some(error);
                        }
                        last_reload = Instant::now();
                    }
                    let translations = &config.translations;
                    let mut actions = Vec::new();
                    let mut missing = std::cell::RefCell::new(std::collections::BTreeSet::new());
                    let was_busy = session.state().busy;
                    if pending_turn.is_some() {
                        session.state().busy = true;
                    }
                    let frame = renderer.frame(input, |ui| {
                        actions = craft_ui::assistant::show(ui, egui::Id::unique("shared-assistant"), session.state(), |text| {
                            translations.get(text).cloned().unwrap_or_else(|| {
                                if missing.borrow().len() < 1024 {
                                    missing.borrow_mut().insert(text.to_owned());
                                }
                                text.to_owned()
                            })
                        });
                    })?;
                    session.state().busy = was_busy;
                    if !missing.get_mut().is_empty() {
                        event(&mut output, Event::Translate { keys: missing.into_inner().into_iter().collect() }, max)?;
                    }
                    for action in actions {
                        match action {
                            Action::Send(prompt) => {
                                pending_turn = Some(prompt.clone());
                                let files = session.state().authorized_files.clone();
                                event(&mut output, Event::BeginTurn { prompt, files }, max)?;
                            }
                            Action::Stop => {
                                if let Some(prompt) = pending_turn.take()
                                    && session.state().draft.is_empty()
                                {
                                    session.state().draft = prompt;
                                }
                                session.stop();
                                event(&mut output, Event::Stop, max)?;
                            }
                            other => {
                                if let Err(error) = session.action(other) {
                                    session.state().error = Some(error);
                                }
                            }
                        }
                    }
                    send(&mut output, HelperMessage::Frame(Box::new(frame)), max)?;
                }
            }
        }
        if session.choose_files {
            session.choose_files = false;
            event(&mut output, Event::ChooseFiles, max)?;
        }
        for pending in session.pending.drain(..) {
            if pending.cancel.is_cancelled() || Instant::now() >= pending.expires {
                let _ = pending.reply.send(Err(AiError::cancelled()));
                continue;
            }
            next_call = next_call.checked_add(1).ok_or("assistant call sequence exhausted")?;
            let id = next_call.to_string();
            let payload = serde_json::to_value(ToolRequest { target: pending.target.clone(), call: pending.call.clone() })
                .map_err(|_| "invalid assistant tool request")?;
            send(&mut output, HelperMessage::ToolCall { id: id.clone(), payload }, max)?;
            if replies.len() >= 128 {
                return Err("too many pending app tool responses".into());
            }
            replies.insert(id, pending);
        }
        let s = session.state();
        event(
            &mut output,
            Event::Status(Status {
                version: env!("CARGO_PKG_VERSION").into(),
                busy: s.busy,
                settings_busy: s.settings.busy,
                settings_open: s.settings_open,
                restart_safe: pending_turn.is_none()
                    && !s.busy
                    && !s.settings.busy
                    && s.draft.is_empty()
                    && s.file_path.is_empty()
                    && s.authorized_files.is_empty()
                    && s.error.is_none()
                    && !s.settings_open
                    && s.settings.connection.key.is_empty()
                    && !s.settings.has_unsaved_edits(),
                authorized_files: s.authorized_files.clone(),
                chat_id: s.active_chat.clone(),
                model: s.chat_route.clone(),
                status: s.status.clone(),
                error: s.error.clone(),
            }),
            max,
        )?;
    }
    session.stop();
    Ok(())
}
fn command(session: &mut Session, name: &str, args: Value) -> Result<(), String> {
    match name {
        "start_turn" => {
            let request: StartTurn = serde_json::from_value(args).map_err(|_| "invalid assistant turn")?;
            if request.prompt.trim().is_empty() {
                return Err("give a nonempty prompt".into());
            }
            session.initial_read = request.initial_read;
            session.start(request.prompt, request.target, request.tools, request.files)
        }
        "stop" => {
            session.stop();
            Ok(())
        }
        "new_chat" => session.action(Action::NewChat),
        "open_chat" => session.action(Action::OpenChat(args.as_str().ok_or("give a chat id")?.into())),
        "delete_chat" => session.action(Action::DeleteChat(args.as_str().ok_or("give a chat id")?.into())),
        "settings" => {
            session.state().settings_open = args.as_bool().unwrap_or(true);

            Ok(())
        }
        "context" => {
            if !session.state().busy {
                session.state().target_label = args.as_str().ok_or("give a document label")?.into();
            }
            Ok(())
        }
        "add_file" => session.action(Action::AddFile(args.as_str().ok_or("give an absolute file path")?.into())),
        "reload_settings" => session.action(Action::ReloadSettings),
        _ => Err("unsupported assistant command; update the app or assistant".into()),
    }
}
