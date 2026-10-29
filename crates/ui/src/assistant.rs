//! Assistant presentation shared by Craft apps. Hosts own inference, persistence, credentials,
//! file grants and document mutation; this module only draws state and returns explicit actions.

use craft_ai_core::assistant::{AssistantRole, Message, MessageRole};
use craft_ai_core::{ConnectionDraft, ConnectionId, Prepared};
use egui::{Id, RichText, Ui};

mod settings;
pub use settings::{RoleEdit, SettingsState, settings};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatSummary {
    pub id: String,
    pub title: String,
    pub interrupted: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    pub label: String,
    pub detail: String,
    pub failed: bool,
}

/// Transient view state. In particular, the connection editor's key is never serialized.
#[derive(Default)]
pub struct State {
    pub messages: Vec<Message>,
    pub history: Vec<ChatSummary>,
    pub active_chat: Option<String>,
    pub draft: String,
    pub busy: bool,
    /// False in a host without native assistant services.
    pub available: bool,
    pub status: Option<String>,
    pub error: Option<String>,
    pub chat_route: String,
    /// Legacy host status; the unified pane displays only chat_route.
    pub control_route: String,
    pub target_label: String,
    pub authorized_files: Vec<String>,
    pub file_path: String,
    pub activity: Vec<Activity>,
    pub settings_open: bool,
    pub settings: SettingsState,
}

/// An explicit user request. Applying it is always the host's responsibility.
#[derive(Debug)]
pub enum Action {
    Send(String),
    Stop,
    NewChat,
    OpenChat(String),
    DeleteChat(String),
    ChooseFiles,
    AddFile(String),
    RemoveFile(String),
    SaveConnection(Prepared),
    DiscoverModels(ConnectionDraft),
    TestConnection(ConnectionDraft),
    VerifyModel { connection: ConnectionDraft, model: String },
    BeginCodexSignIn,
    RefreshCodex,
    ReloadSettings,
    SaveBinding { role: AssistantRole, connection: String, model: String, app_override: bool, manual_tools: bool },
    UseSuiteDefault { role: AssistantRole },
    DeleteConnection(ConnectionId),
}

impl State {
    /// Empty, unavailable and busy submissions preserve the draft.
    pub fn submit(&mut self) -> Option<Action> {
        if !self.available || self.busy || self.draft.trim().is_empty() {
            return None;
        }
        Some(Action::Send(std::mem::take(&mut self.draft)))
    }
}

/// Draw a complete pane. The caller supplies a stable, per-window ID and translates labels;
/// model replies and provider details remain their original text.
pub fn show(ui: &mut Ui, id: Id, state: &mut State, translate: impl Fn(&str) -> String) -> Vec<Action> {
    let mut actions = Vec::new();
    ui.push_id(id, |ui| {
        header(ui, state, &translate, &mut actions);
        if state.settings_open {
            egui::ScrollArea::vertical().id_salt("assistant_settings").show(ui, |ui| {
                actions.extend(settings(ui, id.with("settings"), &mut state.settings, &translate));
            });
            return;
        }
        routes(ui, state, &translate);
        if !state.available {
            ui.label(translate("The assistant is available in the desktop app."));
        }
        if let Some(error) = &state.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if let Some(status) = &state.status {
            ui.horizontal_wrapped(|ui| {
                if state.busy {
                    ui.spinner();
                }
                ui.label(status);
            });
        }
        // Reserve the composer before the transcript so it remains reachable in short panes.
        let height = (ui.available_height() - 145.0).max(45.0);
        egui::ScrollArea::vertical().id_salt("assistant_transcript").max_height(height).auto_shrink([false, false]).stick_to_bottom(true).show(
            ui,
            |ui| {
                transcript(ui, state, &translate);
            },
        );
        ui.separator();
        composer(ui, id, state, &translate, &mut actions);
    });
    actions
}

fn header(ui: &mut Ui, state: &mut State, tr: &impl Fn(&str) -> String, actions: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(!state.busy, egui::Button::new(tr("New chat"))).clicked() {
            actions.push(Action::NewChat);
        }
        if let Some(id) = &state.active_chat
            && ui.add_enabled(!state.busy, egui::Button::new(tr("Delete chat"))).clicked()
        {
            actions.push(Action::DeleteChat(id.clone()));
        }
        if state.settings_open && state.busy && ui.button(tr("Stop")).clicked() {
            actions.push(Action::Stop);
        }
        if ui.selectable_label(state.settings_open, tr("Settings")).clicked() {
            state.settings_open = !state.settings_open;
        }
    });
    ui.add_enabled_ui(!state.busy, |ui| {
        ui.horizontal(|ui| {
            // ComboBox::width is a minimum. Reserve the label first so a long selected title
            // truncates within the remaining width instead of wrapping the label vertically.
            let label = ui.label(tr("History"));
            let title = state.history.iter().find(|chat| Some(&chat.id) == state.active_chat.as_ref()).map(|chat| chat.title.clone());
            crate::ComboBox::from_id_salt("assistant_chats")
                .selected_text(title.unwrap_or_else(|| tr("Saved chats")))
                .width(ui.available_width())
                .truncate()
                .show_ui(ui, |ui| {
                    if state.history.is_empty() {
                        ui.label(tr("No saved chats"));
                    }
                    for chat in &state.history {
                        let selected = Some(&chat.id) == state.active_chat.as_ref();
                        let label = if chat.interrupted { format!("{} · {}", chat.title, tr("Interrupted")) } else { chat.title.clone() };
                        if ui.selectable_label(selected, label).clicked() {
                            actions.push(Action::OpenChat(chat.id.clone()));
                        }
                    }
                })
                .response
                .labelled_by(label.id);
        });
    });
}

fn routes(ui: &mut Ui, state: &State, tr: &impl Fn(&str) -> String) {
    let value = if state.chat_route.is_empty() { tr("Not configured") } else { state.chat_route.clone() };
    ui.small(format!("{}: {value}", tr("Model")));
    if !state.target_label.is_empty() {
        ui.small(craft_i18n::fmt(&tr("Document: {name}"), &[("name", &state.target_label)]));
    }
}

fn transcript(ui: &mut Ui, state: &State, tr: &impl Fn(&str) -> String) {
    if state.messages.is_empty() {
        ui.label(tr("Ask a question or describe a document change."));
    }
    for (index, message) in state.messages.iter().enumerate() {
        if message.role == MessageRole::System {
            continue;
        }
        let speaker = match message.role {
            MessageRole::User => "You",
            MessageRole::Assistant => "Assistant",
            MessageRole::Tool => "Tool result",
            MessageRole::System => "System",
        };
        ui.push_id(index, |ui| {
            if message.role == MessageRole::Tool {
                egui::CollapsingHeader::new(tr(speaker)).show(ui, |ui| {
                    ui.label(&message.content);
                });
            } else {
                ui.label(RichText::new(tr(speaker)).strong());
                ui.label(&message.content);
                for call in &message.tool_calls {
                    ui.small(format!("{}: {}", tr("Tool"), call.name));
                }
            }
            ui.add_space(8.0);
        });
    }
    for (index, activity) in state.activity.iter().enumerate() {
        ui.push_id(("activity", index), |ui| {
            let label =
                if activity.failed { RichText::new(&activity.label).color(ui.visuals().error_fg_color) } else { RichText::new(&activity.label) };
            egui::CollapsingHeader::new(label).show(ui, |ui| {
                ui.label(&activity.detail);
            });
        });
    }
}

fn composer(ui: &mut Ui, id: Id, state: &mut State, tr: &impl Fn(&str) -> String, actions: &mut Vec<Action>) {
    files(ui, state, tr, actions);
    let input = id.with("composer");
    let enter = state.available
        && !state.busy
        && ui.memory(|memory| memory.has_focus(input))
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    ui.add(
        egui::TextEdit::multiline(&mut state.draft)
            .id(input)
            .desired_width(f32::INFINITY)
            .desired_rows(3)
            .hint_text(tr("Message · Shift+Enter for a new line")),
    );
    ui.horizontal(|ui| {
        let send = ui.add_enabled(state.available && !state.busy && !state.draft.trim().is_empty(), egui::Button::new(tr("Send"))).clicked();
        if (send || enter)
            && let Some(action) = state.submit()
        {
            actions.push(action);
        }
        if ui.add_enabled(state.busy, egui::Button::new(tr("Stop"))).clicked() {
            actions.push(Action::Stop);
        }
    });
}

fn files(ui: &mut Ui, state: &mut State, tr: &impl Fn(&str) -> String, actions: &mut Vec<Action>) {
    egui::CollapsingHeader::new(tr("Allowed files")).id_salt("assistant_files").show(ui, |ui| {
        ui.small(tr("Only files you name or choose are available to the assistant."));
        for path in &state.authorized_files {
            ui.horizontal_wrapped(|ui| {
                ui.label(path);
                if ui.add_enabled(!state.busy, egui::Button::new(tr("Remove"))).clicked() {
                    actions.push(Action::RemoveFile(path.clone()));
                }
            });
        }
        ui.add_enabled_ui(state.available && !state.busy, |ui| {
            if ui.button(tr("Choose file…")).clicked() {
                actions.push(Action::ChooseFiles);
            }
            ui.add(egui::TextEdit::singleline(&mut state.file_path).hint_text(tr("Full file path, including a new output file")));
            if ui.add_enabled(!state.file_path.trim().is_empty(), egui::Button::new(tr("Allow path"))).clicked() {
                actions.push(Action::AddFile(std::mem::take(&mut state.file_path)));
            }
        });
    });
}

#[cfg(test)]
mod tests;
