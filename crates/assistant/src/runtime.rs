//! Shared assistant session; documents remain exclusively in the app process.
use craft_ai_core::{
    AppId,
    assistant::{Target, ToolDefinition},
};
use craft_ui::assistant::{Action, State};
pub(crate) mod host;
mod native;

pub struct Session {
    assistant: Panel,
    pub(crate) ctx: Option<egui::Context>,
    pub(crate) app_id: AppId,
    pub(crate) target: Target,
    pub(crate) tools: Vec<ToolDefinition>,
    pub(crate) initial_read: Option<(String, serde_json::Value)>,
    pub(crate) pending: Vec<host::Pending>,
    pub(crate) choose_files: bool,
    pub(crate) translations: std::collections::BTreeMap<String, String>,
    pub(crate) missing_translations: std::cell::RefCell<std::collections::BTreeSet<String>>,
}
#[derive(Default)]
struct Panel {
    pub(crate) widget: State,
    native: native::Native,
    pub(crate) enabled: bool,
}
impl Session {
    pub fn new(app_id: AppId, context: egui::Context) -> Self {
        use craft_fonts_build::EmbeddedFace;
        let faces: &[EmbeddedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));
        let faces: Vec<craft_ui::fonts::CraftFace> = faces.iter().copied().map(Into::into).collect();
        craft_ui::fonts::UiFonts::new(&craft_ui::fonts::I18nScripts).craft_faces(&faces).install(&context);
        Self {
            assistant: Panel { enabled: true, ..Panel::default() },
            ctx: Some(context),
            app_id,
            target: Target { id: String::new(), revision: 0 },
            tools: Vec::new(),
            initial_read: None,
            pending: Vec::new(),
            choose_files: false,
            translations: Default::default(),
            missing_translations: Default::default(),
        }
    }
    pub fn action(&mut self, action: Action) -> Result<(), String> {
        native::action(self, action)
    }
    pub fn poll(&mut self) {
        if let Some(ctx) = self.ctx.clone() {
            native::ensure_loaded(self);
            native::poll(self, &ctx);
        }
    }
    pub fn state(&mut self) -> &mut State {
        &mut self.assistant.widget
    }
    pub fn start(&mut self, prompt: String, target: Target, tools: Vec<ToolDefinition>, files: Vec<String>) -> Result<(), String> {
        if self.assistant.widget.busy {
            return Err("finish or stop the current assistant turn".into());
        }
        if files.len() > 32 || tools.len() > 1024 {
            return Err("assistant request exceeds its file/tool limits".into());
        }
        self.target = target;
        self.tools = tools;
        self.assistant.widget.authorized_files.clear();
        for file in files {
            native::action(self, Action::AddFile(file))?;
        }
        native::action(self, Action::Send(prompt))
    }
    pub fn stop(&mut self) {
        self.assistant.native.cancel();
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn unsaved_settings_edits_defer_component_replacement() {
        let mut settings = craft_ui::assistant::SettingsState::default();
        assert!(!settings.has_unsaved_edits());
        settings.chat.model = "unsaved-model".into();
        assert!(settings.has_unsaved_edits());
        settings.chat.model.clear();
        settings.connection.endpoint = "https://example.invalid/v1".into();
        assert!(settings.has_unsaved_edits());
        settings.connection.endpoint.clear();
        settings.connection.key = "synthetic-unsaved-key".into();
        assert!(settings.has_unsaved_edits());
        settings.connection.key.clear();
        assert!(!settings.has_unsaved_edits());
    }
}
