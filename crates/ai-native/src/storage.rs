//! Locked shared settings and app-local chat files. No document files or credentials live here.

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use craft_ai_core::assistant::{Conversation, RuntimeConfig};
use craft_ai_core::{AiError, AppId, ErrorKind, StoreFile};
use craft_atomic::{NewFilePermissions, WriteOptions};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_HISTORY_BYTES: usize = 8 * 1024 * 1024;
const MAX_CONFIG_BYTES: usize = 64 * 1024;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn storage_error(action: &str, error: impl std::fmt::Display) -> AiError {
    AiError::new(ErrorKind::Internal, format!("Assistant {action} failed: {error}"))
}

fn private_dir(path: &Path) -> Result<(), AiError> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|e| storage_error("folder creation", e))
}

fn locked(path: &Path) -> Result<File, AiError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|e| storage_error("lock", e))?;
    file.lock().map_err(|e| storage_error("lock", e))?;
    Ok(file)
}

fn read_bounded(path: &Path, limit: usize) -> Result<Option<Vec<u8>>, AiError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(storage_error("read", e)),
    };
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1) as u64).read_to_end(&mut bytes).map_err(|e| storage_error("read", e))?;
    if bytes.len() > limit {
        return Err(AiError::invalid("The assistant file exceeds its size limit; it was left untouched."));
    }
    Ok(Some(bytes))
}

fn write_json(path: &Path, value: &impl Serialize, limit: usize) -> Result<(), AiError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| storage_error("serialization", e))?;
    if bytes.len() > limit {
        return Err(AiError::invalid("The assistant file exceeds its size limit; the existing file was preserved."));
    }
    WriteOptions::default().with_new_file_permissions(NewFilePermissions::Private).atomic_write(path, &bytes).map_err(|e| storage_error("save", e))
}

fn component(value: &str) -> Result<&str, AiError> {
    if value.is_empty() || value.len() > 128 || !value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(AiError::invalid("Invalid assistant storage identifier."));
    }
    Ok(value)
}

/// Shared across the suite, with bindings scoped by AppId inside StoreFile.
#[derive(Clone)]
pub struct SharedStore {
    root: PathBuf,
}

impl SharedStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn default_root() -> Result<PathBuf, AiError> {
        if let Some(path) = std::env::var_os("CRAFT_AI_CONFIG_DIR") {
            if path.is_empty() {
                return Err(AiError::invalid("CRAFT_AI_CONFIG_DIR is empty; choose an isolated settings directory."));
            }
            return Ok(PathBuf::from(path));
        }
        dirs::config_dir()
            .map(|path| path.join("Craft").join("ai"))
            .ok_or_else(|| AiError::new(ErrorKind::Internal, "No system configuration directory is available."))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn lock(&self) -> Result<File, AiError> {
        private_dir(&self.root)?;
        locked(&self.root.join("store.lock"))
    }

    fn read(&self) -> Result<(StoreFile, Value), AiError> {
        let Some(bytes) = read_bounded(&self.root.join("connections.json"), craft_ai_core::store::MAX_STORE_BYTES)? else {
            return Ok((StoreFile::default(), serde_json::json!({})));
        };
        let text = std::str::from_utf8(&bytes).map_err(|_| AiError::invalid("Assistant connections are not UTF-8; the file was left untouched."))?;
        let store = StoreFile::parse(text).map_err(|e| AiError::invalid(e.to_string()))?;
        let raw = serde_json::from_str(text).map_err(|e| storage_error("parse", e))?;
        Ok((store, raw))
    }

    pub fn load(&self) -> Result<StoreFile, AiError> {
        let _lock = self.lock()?;
        self.read().map(|(store, _)| store)
    }

    /// Re-read under the lock so another app's connection or binding is never lost.
    pub fn update(&self, change: impl FnOnce(&mut StoreFile) -> Result<(), AiError>) -> Result<StoreFile, AiError> {
        let _lock = self.lock()?;
        let (mut store, mut raw) = self.read()?;
        change(&mut store)?;
        store.validate().map_err(|e| AiError::invalid(e.to_string()))?;
        let encoded = serde_json::to_value(&store).map_err(|e| storage_error("serialization", e))?;
        if let (Some(raw), Some(updated)) = (raw.as_object_mut(), encoded.as_object()) {
            raw.remove("custom_tools");
            for (key, value) in updated {
                raw.insert(key.clone(), value.clone());
            }
        } else {
            raw = encoded;
        }
        write_json(&self.root.join("connections.json"), &raw, craft_ai_core::store::MAX_STORE_BYTES)?;
        Ok(store)
    }

    pub fn config(&self) -> Result<RuntimeConfig, AiError> {
        let _lock = self.lock()?;
        let Some(bytes) = read_bounded(&self.root.join("runtime.json"), MAX_CONFIG_BYTES)? else { return Ok(RuntimeConfig::default()) };
        let config: RuntimeConfig = serde_json::from_slice(&bytes).map_err(|e| storage_error("runtime configuration", e))?;
        config.validate()?;
        Ok(config)
    }

    pub fn update_config(&self, config: &RuntimeConfig) -> Result<(), AiError> {
        config.validate()?;
        let _lock = self.lock()?;
        write_json(&self.root.join("runtime.json"), config, MAX_CONFIG_BYTES)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub interrupted: bool,
}

#[derive(Clone)]
pub struct HistoryStore {
    root: PathBuf,
}

impl HistoryStore {
    pub fn new(root: &Path, app: &AppId) -> Result<Self, AiError> {
        Ok(Self { root: root.join("chats").join(component(app.as_str())?) })
    }

    fn lock(&self) -> Result<File, AiError> {
        private_dir(&self.root)?;
        locked(&self.root.join("history.lock"))
    }

    fn path(&self, id: &str) -> Result<PathBuf, AiError> {
        Ok(self.root.join(format!("{}.json", component(id)?)))
    }

    pub fn create(&self, title: &str) -> Result<Conversation, AiError> {
        let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| storage_error("clock", e))?.as_nanos();
        let id = format!("{time:x}-{:x}-{:x}", std::process::id(), NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let conversation = Conversation { id, title: title.chars().take(160).collect(), messages: Vec::new(), interrupted: false };
        self.save(&conversation)?;
        Ok(conversation)
    }

    pub fn save(&self, conversation: &Conversation) -> Result<(), AiError> {
        let _lock = self.lock()?;
        write_json(&self.path(&conversation.id)?, conversation, MAX_HISTORY_BYTES)
    }

    pub fn load(&self, id: &str) -> Result<Conversation, AiError> {
        let _lock = self.lock()?;
        self.read(id)
    }

    fn read(&self, id: &str) -> Result<Conversation, AiError> {
        let bytes = read_bounded(&self.path(id)?, MAX_HISTORY_BYTES)?.ok_or_else(|| AiError::invalid("This conversation no longer exists."))?;
        let conversation: Conversation = serde_json::from_slice(&bytes).map_err(|e| storage_error("conversation", e))?;
        if conversation.id != id {
            return Err(AiError::invalid("The conversation identifier does not match its file."));
        }
        Ok(conversation)
    }

    pub fn list(&self) -> Result<Vec<ConversationSummary>, AiError> {
        let _lock = self.lock()?;
        let mut summaries = Vec::new();
        for entry in std::fs::read_dir(&self.root).map_err(|e| storage_error("conversation list", e))? {
            let entry = entry.map_err(|e| storage_error("conversation list", e))?;
            if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let stem = entry.path().file_stem().and_then(|s| s.to_str()).map(str::to_owned);
            let Some(stem) = stem else { continue };
            let conversation = self.read(&stem)?;
            if summaries.len() >= 10_000 {
                return Err(AiError::invalid("Too many assistant conversations to list. Archive old conversation files first."));
            }
            summaries.push(ConversationSummary { id: conversation.id, title: conversation.title, interrupted: conversation.interrupted });
        }
        summaries.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(summaries)
    }

    pub fn delete(&self, id: &str) -> Result<(), AiError> {
        let _lock = self.lock()?;
        match std::fs::remove_file(self.path(id)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(storage_error("conversation deletion", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let value = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            Self(std::env::temp_dir().join(format!("craft-ai-storage-{}-{value}", std::process::id())))
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn updates_from_two_apps_preserve_both_bindings() {
        let temp = Temp::new();
        let store = Arc::new(SharedStore::new(temp.0.clone()));
        let handles: Vec<_> = ["wordcraft", "gridcraft"]
            .into_iter()
            .map(|name| {
                let store = store.clone();
                std::thread::spawn(move || {
                    store.update(|data| {
                        data.apps.insert(AppId::new(name).map_err(|e| AiError::invalid(e.to_string()))?, Default::default());
                        Ok(())
                    })
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }
        assert_eq!(store.load().unwrap().apps.len(), 2);
    }

    #[test]
    fn newer_store_is_never_overwritten() {
        let temp = Temp::new();
        private_dir(&temp.0).unwrap();
        let bytes = br#"{"schema_version":999,"connections":[]}"#;
        std::fs::write(temp.0.join("connections.json"), bytes).unwrap();
        let store = SharedStore::new(temp.0.clone());
        assert!(store.update(|_| Ok(())).is_err());
        assert_eq!(std::fs::read(temp.0.join("connections.json")).unwrap(), bytes);
    }

    #[test]
    fn chats_are_app_local_and_interrupted_turns_stay_interrupted() {
        let temp = Temp::new();
        let word = HistoryStore::new(&temp.0, &AppId::new("wordcraft").unwrap()).unwrap();
        let grid = HistoryStore::new(&temp.0, &AppId::new("gridcraft").unwrap()).unwrap();
        let mut chat = word.create("Test").unwrap();
        chat.interrupted = true;
        word.save(&chat).unwrap();
        assert!(word.load(&chat.id).unwrap().interrupted);
        assert!(grid.list().unwrap().is_empty());
        assert!(word.load("../connections").is_err());
        word.delete(&chat.id).unwrap();
        assert!(word.list().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn existing_folder_permissions_are_not_changed() {
        use std::os::unix::fs::PermissionsExt;
        let temp = Temp::new();
        std::fs::create_dir(&temp.0).unwrap();
        std::fs::set_permissions(&temp.0, std::fs::Permissions::from_mode(0o750)).unwrap();
        private_dir(&temp.0).unwrap();
        assert_eq!(std::fs::metadata(&temp.0).unwrap().permissions().mode() & 0o777, 0o750);
        private_dir(&temp.0.join("new")).unwrap();
        assert_eq!(std::fs::metadata(temp.0.join("new")).unwrap().permissions().mode() & 0o777, 0o700);
    }
}
