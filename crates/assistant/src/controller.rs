//! Thin app-side lifecycle. Provider code and the assistant interface live in the child process.
use crate::{
    config::{ComponentConfig, compatibility, default_bridge_limits, default_startup_timeout_secs},
    protocol::{Event, Initialize, RestartState, StartTurn, Status, ToolRequest, ToolResult},
    updates::{self, Candidate},
};
use craft_ai_native::storage::SharedStore;
use craft_assistant_bridge::{Client, EmbeddedPanel, HelperMessage, HostMessage, PROTOCOL};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

type Started = (Client, Option<ComponentConfig>, PathBuf, Option<String>);
enum Update {
    Checked(Option<Candidate>),
    Installed(String),
}

pub enum Request {
    BeginTurn { prompt: String, files: Vec<String> },
    Tool { id: String, request: ToolRequest },
    Stop,
    ChooseFiles,
    Translate { keys: Vec<String> },
}
/// Each host owns one connection. Installed executable paths remain pinned for active sessions.
pub struct Controller {
    app: String,
    translations: BTreeMap<String, String>,
    client: Option<Client>,
    panel: Option<EmbeddedPanel>,
    starting: Option<mpsc::Receiver<Result<Started, String>>>,
    started: Option<Instant>,
    timeout: Duration,
    ready: bool,
    pub status: Status,
    error: Option<String>,
    config: Option<ComponentConfig>,
    sequence: u64,
    generation: u64,
    requests: Vec<Request>,
    update: Option<mpsc::Receiver<Result<Update, String>>>,
    candidate: Option<Candidate>,
    update_notice: Option<String>,
    executable: Option<PathBuf>,
    expected_version: Option<String>,
    restore_chat: Option<String>,
    hidden: bool,
    refreshing: Option<mpsc::Receiver<Result<craft_assistant_install::Installed, String>>>,
    replacement: Option<craft_assistant_install::Installed>,
    restart_barrier: Option<String>,
}
impl Controller {
    pub fn new(app: &str, translations: BTreeMap<String, String>) -> Self {
        Self {
            app: app.into(),
            translations,
            client: None,
            panel: None,
            starting: None,
            started: None,
            timeout: Duration::from_secs(default_startup_timeout_secs()),
            ready: false,
            status: Status::default(),
            error: None,
            config: None,
            sequence: 0,
            generation: 0,
            requests: Vec::new(),
            update: None,
            candidate: None,
            update_notice: None,
            executable: None,
            expected_version: None,
            restore_chat: None,
            hidden: false,
            refreshing: None,
            replacement: None,
            restart_barrier: None,
        }
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn ready(&self) -> bool {
        self.ready
    }
    pub fn dismiss_error(&mut self) {
        if self.ready {
            self.error = None;
            self.status.error = None;
        }
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn owns_keyboard(&self) -> bool {
        !self.hidden && self.panel.as_ref().is_some_and(EmbeddedPanel::owns_keyboard)
    }
    pub fn executable(&self) -> Option<&std::path::Path> {
        self.executable.as_deref()
    }
    pub fn ensure_started(&mut self, ctx: &egui::Context) {
        if self.client.is_some() || self.starting.is_some() || self.error.is_some() {
            return;
        }
        self.timeout = SharedStore::default_root()
            .ok()
            .and_then(|root| ComponentConfig::load(&root).ok())
            .map(|config| config.startup_timeout())
            .unwrap_or(Duration::from_secs(default_startup_timeout_secs()));
        let (sender, receiver) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        let result = std::thread::Builder::new().name("assistant-start".into()).spawn(move || {
            let result = start_component();
            let _ = sender.send(result);
            wake.request_repaint();
        });
        match result {
            Ok(_) => {
                self.starting = Some(receiver);
                self.started = Some(Instant::now());
            }
            Err(error) => self.fail(format!("Could not start the assistant: {error}")),
        }
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        if let Some(receive) = self.refreshing.take() {
            match receive.try_recv() {
                Ok(Ok(installed)) if self.executable.as_ref() != Some(&installed.executable) => {
                    self.replacement = Some(installed);
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => self.update_notice = Some(error),
                Err(mpsc::TryRecvError::Empty) => self.refreshing = Some(receive),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.update_notice = Some("Could not resolve the shared assistant update. The current session was retained.".into())
                }
            }
        }
        if let Some(receive) = self.starting.take() {
            match receive.try_recv() {
                Ok(Ok((mut client, config, executable, expected_version))) => {
                    let limits = config.as_ref().map(|c| c.bridge.clone()).unwrap_or_else(default_bridge_limits);
                    self.timeout =
                        config.as_ref().map(ComponentConfig::startup_timeout).unwrap_or(Duration::from_secs(default_startup_timeout_secs()));
                    let init = serde_json::to_value(Initialize {
                        limits: limits.clone(),
                        restore_chat: self.restore_chat.take(),
                        translations: self.translations.clone(),
                    });
                    let result = init
                        .map_err(|_| "Invalid assistant initialization".to_owned())
                        .and_then(|config| client.send(HostMessage::Initialize { protocol: PROTOCOL, app_id: self.app.clone(), config }))
                        .and_then(|()| EmbeddedPanel::new(limits));
                    match result {
                        Ok(mut panel) => {
                            panel.set_expected_version(expected_version.clone());
                            self.panel = Some(panel);
                            self.client = Some(client);
                            self.config = config;
                            self.executable = Some(executable);
                            self.expected_version = expected_version;
                            self.started = Some(Instant::now());
                        }
                        Err(error) => self.fail(error),
                    }
                }
                Ok(Err(error)) => {
                    self.config = SharedStore::default_root().ok().and_then(|root| ComponentConfig::load(&root).ok());
                    self.fail(error);
                }
                Err(mpsc::TryRecvError::Disconnected) => self.fail("The assistant startup worker stopped".into()),
                Err(mpsc::TryRecvError::Empty) => self.starting = Some(receive),
            }
        }
        if self.started.is_some_and(|time| time.elapsed() >= self.timeout) && !self.ready {
            self.fail("The assistant did not become ready. Reopen it after checking its installation.".into());
        }
        if let (Some(panel), Some(client)) = (&mut self.panel, &mut self.client) {
            panel.poll(ctx, client);
        }
        self.receive_events(ctx);
        self.restart_if_ready();
        let pipe_error = self.client.as_ref().and_then(|c| c.error()).or_else(|| self.panel.as_ref().and_then(|p| p.error())).map(str::to_owned);
        if let Some(error) = pipe_error {
            self.fail(format!("{error}. Inspect completed edits before continuing; nothing will be replayed."));
        }
        if let Some(receive) = self.update.take() {
            match receive.try_recv() {
                Ok(Ok(Update::Checked(candidate))) => {
                    self.update_notice = Some(
                        if candidate.is_some() { "A shared assistant update is ready to install." } else { "The shared assistant is up to date." }
                            .into(),
                    );
                    self.candidate = candidate;
                }
                Ok(Ok(Update::Installed(version))) => {
                    self.update_notice = Some(format!("Assistant {version} installed for all apps. Reopen idle panels to use it."));
                    self.candidate = None;
                }
                Ok(Err(error)) => self.update_notice = Some(error),
                Err(mpsc::TryRecvError::Empty) => self.update = Some(receive),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.update_notice = Some("The assistant update worker stopped. The installed version was retained.".into())
                }
            }
        }
        if self.client.is_some() || self.starting.is_some() || self.update.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
    fn receive_events(&mut self, ctx: &egui::Context) {
        let messages = self.panel.as_mut().map(EmbeddedPanel::poll_events).unwrap_or_default();
        for message in messages {
            if !self.ready && !matches!(message, HelperMessage::Ready { .. }) {
                self.fail("The assistant sent a message before its compatibility handshake.".into());
                break;
            }
            match message {
                HelperMessage::Ready { protocol, version, capabilities } => {
                    if self.ready
                        || self.expected_version.as_ref().is_some_and(|expected| expected != &version)
                        || protocol != PROTOCOL
                        || !capabilities.iter().any(|c| c == "tools-v1")
                    {
                        self.fail("The installed assistant is incompatible. Update the app or assistant.".into());
                        break;
                    }
                    self.generation = self.generation.saturating_add(1);
                    self.status.version = version;
                    self.ready = true;
                    self.started = None;
                }
                HelperMessage::Event(value) => match serde_json::from_value::<Event>(value) {
                    Ok(Event::BeginTurn { prompt, files }) => {
                        self.refreshing = None;
                        self.replacement = None;
                        self.restart_barrier = None;
                        self.requests.push(Request::BeginTurn { prompt, files });
                    }
                    Ok(Event::Stop) => self.requests.push(Request::Stop),
                    Ok(Event::ChooseFiles) => {
                        self.refreshing = None;
                        self.replacement = None;
                        self.restart_barrier = None;
                        self.requests.push(Request::ChooseFiles);
                    }
                    Ok(Event::Translate { keys }) => self.requests.push(Request::Translate { keys }),
                    Ok(Event::Status(status)) => self.status = status,
                    Ok(Event::Result { id, result }) if self.restart_barrier.as_ref() == Some(&id) => {
                        self.restart_barrier = None;
                        match result.and_then(|value| serde_json::from_value::<RestartState>(value).map_err(|_| "invalid restart state".into())) {
                            Ok(state) if state.safe => {
                                self.restore_chat = state.chat_id;
                                self.replacement = None;
                                self.client = None;
                                self.panel = None;
                                self.ready = false;
                                self.error = None;
                                self.ensure_started(ctx);
                                return;
                            }
                            Ok(_) => {
                                self.replacement = None;
                                self.update_notice =
                                    Some("The current assistant has unsaved input or active work. It was retained; reopen after finishing.".into());
                            }
                            Err(error) => {
                                self.replacement = None;
                                self.update_notice = Some(error);
                            }
                        }
                    }
                    Ok(Event::Result { result: Err(error), .. }) => {
                        self.error = Some(error);
                        self.requests.push(Request::Stop);
                    }
                    Ok(Event::Result { .. }) => {
                        self.error = None;
                    }
                    Err(_) => {
                        self.fail("Invalid assistant service event; update the assistant.".into());
                        break;
                    }
                },
                HelperMessage::ToolCall { id, payload } => match serde_json::from_value(payload) {
                    Ok(request) => self.requests.push(Request::Tool { id, request }),
                    Err(_) => {
                        self.fail("Invalid assistant tool request".into());
                        break;
                    }
                },
                HelperMessage::Error { message } => {
                    self.fail(message);
                    break;
                }
                HelperMessage::Frame(_) => {}
            }
        }
    }
    fn fail(&mut self, error: String) {
        self.error = Some(error);
        self.status.busy = false;
        self.ready = false;
        self.client = None;
        self.panel = None;
        self.starting = None;
        self.started = None;
        self.replacement = None;
        self.restart_barrier = None;
        self.refreshing = None;
        self.requests.clear();
        self.requests.push(Request::Stop);
    }
    pub fn requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }
    pub fn command(&mut self, name: &str, args: Value) -> Result<(), String> {
        if self.refreshing.is_some() || self.replacement.is_some() || self.restart_barrier.is_some() {
            return Err("The assistant is checking its installed version; try again when ready.".into());
        }
        if !self.ready {
            return Err("The shared assistant is not ready; open its panel and check its installation.".into());
        }
        self.sequence = self.sequence.checked_add(1).ok_or("assistant command sequence exhausted")?;
        self.client.as_mut().ok_or("assistant is unavailable")?.send(HostMessage::Command { id: self.sequence.to_string(), name: name.into(), args })
    }
    pub fn start_turn(&mut self, turn: StartTurn) -> Result<(), String> {
        if self.ready {
            self.error = None;
        }
        let args = serde_json::to_value(turn).map_err(|_| "invalid assistant turn")?;
        self.command("start_turn", args)
    }
    pub fn tool_result(&mut self, id: String, result: ToolResult) -> Result<(), String> {
        self.client
            .as_mut()
            .ok_or("assistant is unavailable")?
            .send(HostMessage::ToolResult { id, result: serde_json::to_value(result).map_err(|_| "invalid tool result")? })
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        if self.hidden {
            self.hidden = false;
            self.refresh_installed(ui.ctx());
        }
        self.ensure_started(ui.ctx());
        self.poll(ui.ctx());
        ui.horizontal_wrapped(|ui| {
            ui.small(format!("Shared assistant {}", if self.status.version.is_empty() { "—" } else { &self.status.version }));
            if self.config.is_some() && ui.add_enabled(self.update.is_none(), egui::Button::new("Check for updates")).clicked() {
                self.check_update(ui.ctx());
            }
        });
        if let Some(notice) = &self.update_notice {
            ui.small(notice);
        }
        if let Some(candidate) = &self.candidate {
            let title = format!("Install assistant {} for all apps", candidate.version);
            if ui.add_enabled(self.update.is_none() && !self.status.busy && !self.status.settings_busy, egui::Button::new(title)).clicked() {
                self.install_update(ui.ctx());
            }
        }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if self.error.is_some() && !self.status.busy && ui.button("Reload component configuration").clicked() {
            self.error = None;
            if self.client.is_none() {
                self.ensure_started(ui.ctx());
            }
        }
        if let (Some(panel), Some(client)) = (&mut self.panel, &mut self.client) {
            ui.add_enabled_ui(self.refreshing.is_none() && self.replacement.is_none() && self.restart_barrier.is_none(), |ui| panel.show(ui, client));
        } else if self.starting.is_some() {
            ui.spinner();
            ui.label("Starting shared assistant…");
        } else {
            ui.label("Configure Craft/ai/component.json, then reload. The assistant is shared by all Craft apps.");
        }
        self.receive_events(ui.ctx());
    }
    /// Called while the app's assistant pane is hidden. Active turns continue on their pinned executable.
    pub fn hidden(&mut self) {
        self.hidden = true;
    }
    pub fn translations(&mut self, entries: BTreeMap<String, String>) -> Result<(), String> {
        self.translations.extend(entries.clone());
        self.command("translations", serde_json::to_value(entries).map_err(|_| "invalid translations")?)
    }
    pub fn language_changed(&mut self) -> Result<(), String> {
        self.translations.clear();
        self.command("reset_translations", Value::Null)
    }
    fn restart_if_ready(&mut self) {
        if self.requests.iter().any(|request| matches!(request, Request::BeginTurn { .. } | Request::Tool { .. } | Request::ChooseFiles)) {
            return;
        }
        if self.replacement.is_none() || self.restart_barrier.is_some() || self.panel.as_ref().is_none_or(EmbeddedPanel::input_pending) {
            return;
        }
        let Some(client) = &mut self.client else {
            self.replacement = None;
            return;
        };
        self.sequence = match self.sequence.checked_add(1) {
            Some(next) => next,
            None => {
                self.replacement = None;
                return;
            }
        };
        let id = self.sequence.to_string();
        match client.send(HostMessage::Command { id: id.clone(), name: "restart_state".into(), args: Value::Null }) {
            Ok(()) => self.restart_barrier = Some(id),
            Err(error) => {
                self.replacement = None;
                self.update_notice = Some(error);
            }
        }
    }
    fn refresh_installed(&mut self, ctx: &egui::Context) {
        if !self.ready || self.refreshing.is_some() {
            return;
        }
        if !self.status.restart_safe {
            self.update_notice = Some("Shared updates will be used after drafts and settings edits are finished and the app restarts.".into());
            return;
        }
        let Some(config) = self.config.clone() else { return };
        let wake = ctx.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        match std::thread::Builder::new().name("assistant-refresh".into()).spawn(move || {
            let result = craft_assistant_install::Store::new(config.installation).and_then(|store| store.resolve(&compatibility()));
            let _ = sender.send(result);
            wake.request_repaint();
        }) {
            Ok(_) => self.refreshing = Some(receiver),
            Err(error) => self.update_notice = Some(format!("Could not check the installed assistant: {error}")),
        }
    }
    /// Explicit user/automation request; never checks an unconfigured source.
    pub fn check_for_updates(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.config.is_none() {
            return Err("Configure the shared assistant component source first.".into());
        }
        if self.update.is_some() {
            return Err("An assistant update operation is already running.".into());
        }
        self.check_update(ctx);
        Ok(())
    }
    /// Approving this action installs only the exact candidate from the preceding check.
    pub fn install_available_update(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.status.busy || self.status.settings_busy {
            return Err("Finish or stop the active assistant operation before installing an update.".into());
        }
        if self.update.is_some() || self.candidate.is_none() {
            return Err("Check for an available assistant update before installing it.".into());
        }
        self.install_update(ctx);
        Ok(())
    }
    pub fn update_status(&self) -> Value {
        serde_json::json!({"installed":self.status.version,"available":self.candidate.as_ref().map(|c|c.version.to_string()),"busy":self.update.is_some(),"notice":self.update_notice,"source":self.config.as_ref().map(|c|&c.source.repo)})
    }
    pub fn reload_component(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.status.busy || self.status.settings_busy {
            return Err("Finish or stop the active assistant operation first.".into());
        }
        self.error = None;
        if self.client.is_none() {
            self.ensure_started(ctx);
        } else {
            self.refresh_installed(ctx);
        }
        Ok(())
    }
    fn check_update(&mut self, ctx: &egui::Context) {
        let Some(config) = self.config.clone() else { return };
        let current = (!self.status.version.is_empty()).then(|| self.status.version.clone());
        self.update_job(ctx, move || updates::check(&config, current.as_deref()).map(Update::Checked));
    }
    fn install_update(&mut self, ctx: &egui::Context) {
        if self.status.busy || self.status.settings_busy {
            self.update_notice = Some("Finish or stop the active assistant operation before installing an update.".into());
            return;
        }
        let (Some(config), Some(candidate)) = (self.config.clone(), self.candidate.clone()) else { return };
        self.update_job(ctx, move || updates::install(&config, &candidate).map(|i| Update::Installed(i.version.to_string())));
    }
    fn update_job(&mut self, ctx: &egui::Context, work: impl FnOnce() -> Result<Update, String> + Send + 'static) {
        if self.update.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        match std::thread::Builder::new().name("assistant-update".into()).spawn(move || {
            let _ = sender.send(work());
            wake.request_repaint();
        }) {
            Ok(_) => {
                self.update = Some(receiver);
                self.update_notice = Some("Checking/installing shared assistant…".into());
            }
            Err(error) => self.update_notice = Some(format!("Could not start update: {error}")),
        }
    }
}
fn start_component() -> Result<Started, String> {
    let root = SharedStore::default_root().map_err(|e| e.to_string())?;
    // Explicit, debug-only development launch; shipping builds always require signed installation.
    #[cfg(debug_assertions)]
    if let Some(executable) = std::env::var_os("CRAFT_ASSISTANT_EXECUTABLE") {
        let path = PathBuf::from(executable);
        if !path.is_absolute() || !path.is_file() {
            return Err("CRAFT_ASSISTANT_EXECUTABLE must name an existing absolute executable path".into());
        }
        return Ok((Client::spawn(&path, default_bridge_limits())?, None, path, None));
    }
    let config = ComponentConfig::load(&root)?;
    let installed = craft_assistant_install::Store::new(config.installation.clone())?.resolve(&compatibility())?;
    Ok((Client::spawn(&installed.executable, config.bridge.clone())?, Some(config), installed.executable, Some(installed.version.to_string())))
}

#[cfg(test)]
#[path = "controller_tests.rs"]
mod tests;
