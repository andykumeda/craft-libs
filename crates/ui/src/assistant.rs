//! Assistant presentation shared by Craft apps. Hosts own inference, persistence, credentials,
//! file grants and document mutation; this module only draws state and returns explicit actions.

use craft_ai_core::assistant::{AssistantRole, Message, MessageRole};
use craft_ai_core::{ConnectionDraft, ConnectionId, Prepared};
use egui::{Id, Ui};

mod interaction;
mod settings;
pub use interaction::{Interaction, InteractionForm};
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
    pub interaction: Option<Interaction>,
    pub interaction_form: InteractionForm,
}

/// An explicit user request. Applying it is always the host's responsibility.
pub enum Action {
    Send(String),
    Respond { id: u64, response: serde_json::Value },
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

impl std::fmt::Debug for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Send(_) => "Send",
            Self::Respond { id, .. } => return f.debug_struct("Respond").field("id", id).finish_non_exhaustive(),
            Self::Stop => "Stop",
            Self::NewChat => "NewChat",
            Self::OpenChat(_) => "OpenChat",
            Self::DeleteChat(_) => "DeleteChat",
            Self::ChooseFiles => "ChooseFiles",
            Self::AddFile(_) => "AddFile",
            Self::RemoveFile(_) => "RemoveFile",
            Self::SaveConnection(_) => "SaveConnection",
            Self::DiscoverModels(_) => "DiscoverModels",
            Self::TestConnection(_) => "TestConnection",
            Self::VerifyModel { .. } => "VerifyModel",
            Self::BeginCodexSignIn => "BeginCodexSignIn",
            Self::RefreshCodex => "RefreshCodex",
            Self::ReloadSettings => "ReloadSettings",
            Self::SaveBinding { .. } => "SaveBinding",
            Self::UseSuiteDefault { .. } => "UseSuiteDefault",
            Self::DeleteConnection(_) => "DeleteConnection",
        };
        f.write_str(name)
    }
}

impl State {
    /// Empty, unavailable and busy submissions preserve the draft.
    pub fn submit(&mut self) -> Option<Action> {
        if !self.available || self.busy || self.interaction.is_some() || self.draft.trim().is_empty() {
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
        if state.interaction.is_none() {
            state.interaction_form = InteractionForm::default();
        }
        header(ui, state, &translate, &mut actions);
        if let Some(request) = &state.interaction {
            if let Some(action) = interaction::show(ui, request, &mut state.interaction_form, &translate) {
                actions.push(action);
            }
            return;
        }
        if state.settings_open {
            egui::ScrollArea::vertical().id_salt("assistant_settings").show(ui, |ui| {
                actions.extend(settings(ui, id.with("settings"), &mut state.settings, &translate));
            });
            return;
        }
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
        ui.add_space(8.0);
        let focused = ui.memory(|memory| memory.has_focus(id.with("composer")));
        let outline = if focused { ui.visuals().selection.stroke } else { ui.visuals().widgets.noninteractive.fg_stroke };
        egui::Frame::new().fill(ui.visuals().extreme_bg_color).stroke(outline).corner_radius(12.0).inner_margin(10.0).show(ui, |ui| {
            composer(ui, id, state, &translate, &mut actions);
        });
    });
    actions
}

fn header(ui: &mut Ui, state: &mut State, tr: &impl Fn(&str) -> String, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!state.busy, |ui| {
            ui.set_max_width((ui.available_width() - 40.0).max(1.0));
            if ui.button(tr("New chat")).on_hover_text(tr("Start a new conversation")).clicked() {
                actions.push(Action::NewChat);
            }
            let history = crate::ComboBox::from_id_salt("assistant_history")
                .selected_text(tr("Chat history"))
                .width(108.0)
                .truncate()
                .show_ui(ui, |ui| {
                    ui.set_max_width(260.0);
                    if state.history.is_empty() {
                        ui.label(tr("No saved chats"));
                    }
                    for chat in &state.history {
                        let label = if chat.interrupted { format!("{} · {}", chat.title, tr("Interrupted")) } else { chat.title.clone() };
                        if ui.selectable_label(Some(&chat.id) == state.active_chat.as_ref(), label).clicked() {
                            actions.push(Action::OpenChat(chat.id.clone()));
                            ui.close();
                        }
                    }
                    if let Some(id) = &state.active_chat {
                        ui.separator();
                        if ui.button(tr("Delete chat")).clicked() {
                            actions.push(Action::DeleteChat(id.clone()));
                            ui.close();
                        }
                    }
                })
                .response;
            history.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, ui.is_enabled(), tr("Chat history")));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let response = ui.add(egui::Button::new(" ").min_size(egui::vec2(32.0, 32.0)).selected(state.settings_open));
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, state.settings_open, tr("Settings")));
            // Original geometric gear: no font glyph or external image is required.
            let center = response.rect.center();
            let stroke = ui.style().interact(&response).fg_stroke;
            ui.painter().circle_stroke(center, 6.0, stroke);
            ui.painter().circle_stroke(center, 2.5, stroke);
            for tooth in 0..8 {
                let direction = egui::Vec2::angled(tooth as f32 * std::f32::consts::TAU / 8.0);
                ui.painter().line_segment([center + direction * 6.0, center + direction * 9.0], stroke);
            }
            if response.on_hover_text(tr("Settings")).clicked() {
                state.settings_open = !state.settings_open;
            }
            if state.settings_open && state.busy && ui.button(tr("Stop")).clicked() {
                actions.push(Action::Stop);
            }
        });
    });
}

fn transcript(ui: &mut Ui, state: &State, tr: &impl Fn(&str) -> String) {
    if state.messages.is_empty() {
        ui.label(tr("Ask a question or describe a document change."));
    }
    for (index, message) in state.messages.iter().enumerate() {
        if matches!(message.role, MessageRole::System | MessageRole::Tool) {
            continue;
        }
        if message.role == MessageRole::Assistant && message.content.is_empty() {
            continue;
        }
        ui.push_id(index, |ui| {
            if message.role == MessageRole::User {
                let width = (ui.available_width() * 0.85 - 16.0).max(1.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(12.0).inner_margin(8.0).show(ui, |ui| {
                        ui.set_width(width);
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.add(egui::Label::new(&message.content).wrap());
                        });
                    });
                });
            } else {
                ui.add(egui::Label::new(&message.content).wrap());
            }
            ui.add_space(8.0);
        });
    }
}

fn composer(ui: &mut Ui, id: Id, state: &mut State, tr: &impl Fn(&str) -> String, actions: &mut Vec<Action>) {
    for path in &state.authorized_files {
        ui.horizontal_wrapped(|ui| {
            ui.label(path);
            if ui.add_enabled(!state.busy, egui::Button::new("×")).on_hover_text(tr("Remove")).clicked() {
                actions.push(Action::RemoveFile(path.clone()));
            }
        });
    }
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
            .frame(egui::Frame::NONE)
            .hint_text(tr("Message · Shift+Enter for a new line")),
    );
    ui.horizontal(|ui| {
        ui.add_enabled_ui(state.available && !state.busy, |ui| {
            let response = ui.menu_button("+", |ui| files(ui, state, tr, actions)).response;
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tr("Add files")));
            response.on_hover_text(tr("Add files"));
        });
        ui.add_enabled_ui(!state.busy && !state.settings.busy, |ui| {
            ui.set_max_width((ui.available_width() - 50.0).clamp(40.0, 160.0));
            let value = if state.chat_route.is_empty() { tr("Choose model") } else { state.chat_route.clone() };
            let label = egui::RichText::new(value);
            let response = crate::ComboBox::from_id_salt("composer_model")
                .selected_text(label)
                .width(ui.available_width())
                .truncate()
                .show_ui(ui, |ui| {
                    ui.set_max_width(260.0);
                    ui.small(tr(if state.settings.app_id.is_some() {
                        "Changes affect this app only."
                    } else {
                        "Changes set the default for all Craft apps."
                    }));
                    for connection in state.settings.store.connections.iter().filter(|connection| connection.enabled) {
                        ui.strong(&connection.label);
                        for model in &connection.models {
                            let verified = model.capabilities.iter().any(|spec| spec.features.tools && spec.features.streaming);
                            let selected = state.settings.chat.connection == connection.id.as_str() && state.settings.chat.model == model.id.as_str();
                            let response = ui.add_enabled(verified, egui::Button::selectable(selected, &model.label));
                            if response.on_hover_text(tr(if verified { "Choose model" } else { "Tools unverified" })).clicked() {
                                actions.push(Action::SaveBinding {
                                    role: AssistantRole::Chat,
                                    connection: connection.id.as_str().into(),
                                    model: model.id.as_str().into(),
                                    app_override: state.settings.app_id.is_some(),
                                    manual_tools: false,
                                });
                                ui.close();
                            }
                        }
                    }
                    ui.separator();
                    if ui.button(tr("Configure models in settings")).clicked() {
                        state.settings_open = true;
                        ui.close();
                    }
                })
                .response;
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tr("Choose model")));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if state.busy {
                if ui.button(tr("Stop")).clicked() {
                    actions.push(Action::Stop);
                }
            } else {
                let response =
                    ui.add_enabled(state.available && !state.draft.trim().is_empty(), egui::Button::new(" ").min_size(egui::vec2(28.0, 28.0)));
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tr("Send")));
                // Paint the arrow so a minimal host font stack cannot turn Send into a missing glyph.
                let center = response.rect.center();
                let stroke = ui.style().interact(&response).fg_stroke;
                ui.painter().line_segment([center + egui::vec2(0.0, 6.0), center - egui::vec2(0.0, 6.0)], stroke);
                ui.painter().line_segment([center + egui::vec2(-5.0, -1.0), center - egui::vec2(0.0, 6.0)], stroke);
                ui.painter().line_segment([center + egui::vec2(5.0, -1.0), center - egui::vec2(0.0, 6.0)], stroke);
                let send = response.on_hover_text(tr("Send")).clicked();
                if (send || enter)
                    && let Some(action) = state.submit()
                {
                    actions.push(action);
                }
            }
        });
    });
}

fn files(ui: &mut Ui, state: &mut State, tr: &impl Fn(&str) -> String, actions: &mut Vec<Action>) {
    ui.set_max_width(260.0);
    if ui.button(tr("Choose file…")).clicked() {
        actions.push(Action::ChooseFiles);
        ui.close();
    }
    ui.add(egui::TextEdit::singleline(&mut state.file_path).hint_text(tr("Full file path, including a new output file")));
    if ui.add_enabled(!state.file_path.trim().is_empty(), egui::Button::new(tr("Add path"))).clicked() {
        actions.push(Action::AddFile(std::mem::take(&mut state.file_path)));
        ui.close();
    }
}

#[cfg(test)]
mod tests;
