use super::{Action, AssistantRole};
use craft_ai_core::{AppId, ConnectionDraft, StoreFile};
use egui::{Id, Ui};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoleEdit {
    pub connection: String,
    pub model: String,
    pub app_override: bool,
    pub manual_tools: bool,
}

/// Settings copied from the shared store plus unsaved form buffers. The host reloads the store
/// after an acknowledged save; clicking a button never optimistically changes saved settings.
#[derive(Default)]
pub struct SettingsState {
    pub store: StoreFile,
    pub app_id: Option<AppId>,
    pub connection: ConnectionDraft,
    pub chat: RoleEdit,
    pub control: RoleEdit,
    pub preset: String,
    pub probe_model: String,
    pub busy: bool,
    pub status: Option<String>,
    pub error: Option<String>,
    pub codex_status: String,
}

impl SettingsState {
    /// Load acknowledged settings. Connection editing buffers are preserved across refreshes.
    pub fn set_store(&mut self, store: StoreFile, app_id: Option<AppId>) {
        self.store = store;
        self.app_id = app_id;
        self.chat = self.saved_role(AssistantRole::Chat);
        self.control = self.saved_role(AssistantRole::Control);
    }

    fn saved_role(&self, role: AssistantRole) -> RoleEdit {
        let local = self.app_id.as_ref().and_then(|id| self.store.apps.get(id));
        let own = local.and_then(|settings| settings.overrides.iter().find(|binding| binding.role.as_str() == role.id()));
        let chosen = own.or_else(|| self.store.defaults.iter().find(|binding| binding.role.as_str() == role.id()));
        chosen.map_or_else(RoleEdit::default, |binding| RoleEdit {
            connection: binding.connection.as_str().into(),
            model: binding.model.as_str().into(),
            app_override: own.is_some(),
            manual_tools: false,
        })
    }

    /// Prepare a save using shared validation. The host clears the key after an acknowledged save.
    pub fn prepare_connection(&mut self) -> Option<Action> {
        match self.connection.prepare(&self.store.connections) {
            Ok(prepared) => {
                self.error = None;
                Some(Action::SaveConnection(prepared))
            }
            Err(error) => {
                self.error = Some(error.to_string());
                None
            }
        }
    }
}

/// Shared model/connection settings, embeddable in an app's Options page or its assistant pane.
pub fn settings(ui: &mut Ui, id: Id, state: &mut SettingsState, translate: impl Fn(&str) -> String) -> Vec<Action> {
    let mut actions = Vec::new();
    ui.push_id(id, |ui| {
        ui.heading(translate("Assistant settings"));
        ui.small(translate("Connections are shared across Craft apps. Each app can override its chat and control models."));
        ui.small(translate("Document requests may send relevant content to both selected providers."));
        if let Some(error) = &state.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if let Some(status) = &state.status {
            ui.label(status);
        }
        if ui.add_enabled(!state.busy, egui::Button::new(translate("Reload settings"))).clicked() {
            actions.push(Action::ReloadSettings);
        }
        ui.add_enabled_ui(!state.busy, |ui| {
            roles(ui, state, &translate, &mut actions);
            ui.separator();
            connections(ui, state, &translate, &mut actions);
            ui.separator();
            codex(ui, state, &translate, &mut actions);
        });
        if state.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                if ui.button(translate("Stop")).clicked() {
                    actions.push(Action::Stop);
                }
            });
        }
    });
    actions
}

fn roles(ui: &mut Ui, state: &mut SettingsState, tr: &impl Fn(&str) -> String, actions: &mut Vec<Action>) {
    let can_override = state.app_id.is_some();
    for (role, edit) in [(AssistantRole::Chat, &mut state.chat), (AssistantRole::Control, &mut state.control)] {
        ui.push_id(role.id(), |ui| {
            ui.strong(tr(match role {
                AssistantRole::Chat => "General chat",
                AssistantRole::Control => "App control",
            }));
            let picked = state.store.connections.iter().find(|connection| connection.id.as_str() == edit.connection);
            crate::ComboBox::new("connection", tr("Connection"))
                .selected_text(picked.map(|connection| connection.label.clone()).unwrap_or_else(|| tr("Choose connection")))
                .truncate()
                .show_ui(ui, |ui| {
                    for connection in state.store.connections.iter().filter(|connection| connection.enabled) {
                        if ui.selectable_label(connection.id.as_str() == edit.connection, &connection.label).clicked() {
                            edit.connection = connection.id.as_str().into();
                            edit.model.clear();
                            edit.manual_tools = false;
                        }
                    }
                });
            let connection = state.store.connections.iter().find(|connection| connection.id.as_str() == edit.connection);
            crate::ComboBox::new("model", tr("Model"))
                .selected_text(if edit.model.is_empty() { tr("Choose model") } else { edit.model.clone() })
                .truncate()
                .show_ui(ui, |ui| {
                    if let Some(connection) = connection {
                        for model in &connection.models {
                            let tools = model.capabilities.iter().any(|spec| spec.features.tools && spec.features.streaming);
                            let label = if tools { model.label.clone() } else { format!("{} · {}", model.label, tr("Tools unverified")) };
                            if ui.selectable_label(model.id.as_str() == edit.model, label).clicked() {
                                edit.model = model.id.as_str().into();
                                edit.manual_tools = false;
                            }
                        }
                    }
                });
            if ui.add(egui::TextEdit::singleline(&mut edit.model).hint_text(tr("Or enter a model ID"))).changed() {
                edit.manual_tools = false;
            }
            let verified = connection
                .and_then(|connection| connection.models.iter().find(|model| model.id.as_str() == edit.model))
                .is_some_and(|model| model.capabilities.iter().any(|spec| spec.features.tools && spec.features.streaming));
            if !verified {
                ui.checkbox(&mut edit.manual_tools, tr("I confirm this model supports streaming and tool calls"));
            }
            ui.add_enabled(can_override, egui::Checkbox::new(&mut edit.app_override, tr("Override for this app")));
            ui.small(tr(if edit.app_override { "Changes affect this app only." } else { "Changes set the default for all Craft apps." }));
            let ready = connection.is_some_and(|connection| connection.enabled) && !edit.model.trim().is_empty() && (verified || edit.manual_tools);
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(ready, egui::Button::new(tr("Apply model"))).clicked() {
                    actions.push(Action::SaveBinding {
                        role,
                        connection: edit.connection.clone(),
                        model: edit.model.trim().into(),
                        app_override: edit.app_override,
                        manual_tools: edit.manual_tools,
                    });
                }
                if ui.add_enabled(can_override, egui::Button::new(tr("Use suite default"))).clicked() {
                    actions.push(Action::UseSuiteDefault { role });
                }
            });
            ui.add_space(8.0);
        });
    }
}

fn connections(ui: &mut Ui, state: &mut SettingsState, tr: &impl Fn(&str) -> String, actions: &mut Vec<Action>) {
    ui.strong(tr("Connections"));
    ui.horizontal_wrapped(|ui| {
        crate::ComboBox::new("saved_connection", tr("Edit connection"))
            .selected_text(
                state
                    .connection
                    .editing
                    .as_ref()
                    .and_then(|id| state.store.connections.iter().find(|connection| &connection.id == id))
                    .map(|connection| connection.label.clone())
                    .unwrap_or_else(|| tr("New connection")),
            )
            .truncate()
            .show_ui(ui, |ui| {
                if ui.selectable_label(state.connection.editing.is_none(), tr("New connection")).clicked() {
                    state.connection = ConnectionDraft::default();
                    state.preset.clear();
                }
                for connection in state.store.connections.iter().filter(|connection| connection.provider != "codex") {
                    if ui.selectable_label(state.connection.editing.as_ref() == Some(&connection.id), &connection.label).clicked() {
                        state.connection = ConnectionDraft::from_connection(connection);
                        state.preset.clear();
                    }
                }
            });
        if let Some(id) = &state.connection.editing
            && ui.button(tr("Delete connection")).clicked()
        {
            actions.push(Action::DeleteConnection(id.clone()));
        }
    });
    crate::ComboBox::new("connection_preset", tr("Provider preset"))
        .selected_text(
            craft_ai_core::PRESETS
                .iter()
                .find(|preset| preset.key == state.preset)
                .map(|preset| tr(preset.name))
                .unwrap_or_else(|| tr("Provider preset")),
        )
        .show_ui(ui, |ui| {
            for preset in
                craft_ai_core::PRESETS.iter().filter(|preset| ["openai", "openrouter", "ollama", "lmstudio", "custom"].contains(&preset.key))
            {
                if ui.selectable_label(preset.key == state.preset, tr(preset.name)).clicked() {
                    state.connection = ConnectionDraft::from_preset(preset);
                    state.preset = preset.key.into();
                }
            }
        });
    field(ui, tr("Name"), &mut state.connection.label);
    field(ui, tr("API base URL"), &mut state.connection.endpoint);
    ui.checkbox(&mut state.connection.enabled, tr("Connection enabled"));
    ui.checkbox(&mut state.connection.uses_key, tr("Use API key"));
    if state.connection.uses_key {
        let label = ui.label(tr("API key"));
        ui.add(egui::TextEdit::singleline(&mut state.connection.key).password(true).hint_text(tr("Leave empty to keep a saved key")))
            .labelled_by(label.id);
        ui.small(tr("Stored in the operating system credential store."));
        field(ui, tr("API key header (empty uses Bearer authorization)"), &mut state.connection.key_header);
    }
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr("Save connection")).clicked()
            && let Some(action) = state.prepare_connection()
        {
            actions.push(action);
        }
        if ui.button(tr("Discover models")).clicked() {
            actions.push(Action::DiscoverModels(state.connection.clone()));
        }
        if ui.button(tr("Test connection")).clicked() {
            actions.push(Action::TestConnection(state.connection.clone()));
        }
    });
    field(ui, tr("Model to verify"), &mut state.probe_model);
    if ui.add_enabled(!state.probe_model.trim().is_empty(), egui::Button::new(tr("Verify tool support"))).clicked() {
        actions.push(Action::VerifyModel { connection: state.connection.clone(), model: state.probe_model.trim().into() });
    }
    ui.small(tr("Verification sends a small synthetic request; provider charges may apply."));
}

fn codex(ui: &mut Ui, state: &SettingsState, tr: &impl Fn(&str) -> String, actions: &mut Vec<Action>) {
    ui.strong(tr("Codex account"));
    ui.small(tr("Craft uses a separate sign-in and configuration from your coding tools."));
    if !state.codex_status.is_empty() {
        ui.label(&state.codex_status);
    }
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr("Sign in to Codex")).clicked() {
            actions.push(Action::BeginCodexSignIn);
        }
        if ui.button(tr("Refresh Codex models")).clicked() {
            actions.push(Action::RefreshCodex);
        }
    });
}

fn field(ui: &mut Ui, label: String, value: &mut String) {
    let label = ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY)).labelled_by(label.id);
}
