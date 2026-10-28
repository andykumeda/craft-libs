//! Credentials stay in the platform store, outside serializable assistant settings.

use craft_ai_core::{AiError, ErrorKind, SecretRef, SecretString};

pub trait SecretStore: Send + Sync {
    fn get(&self, reference: &SecretRef) -> Result<SecretString, AiError>;
    fn set(&self, reference: &SecretRef, value: &SecretString) -> Result<(), AiError>;
    fn delete(&self, reference: &SecretRef) -> Result<(), AiError>;
}

/// All Craft apps use the same service; entries are addressed by the shared SecretRef.
#[derive(Default)]
pub struct KeyringStore;

const SERVICE: &str = "org.craft-apps.assistant";

impl KeyringStore {
    fn entry(reference: &SecretRef) -> Result<keyring::Entry, AiError> {
        keyring::Entry::new(SERVICE, reference.as_str()).map_err(|_| unavailable())
    }
}

fn unavailable() -> AiError {
    AiError::new(ErrorKind::Auth, "The system credential store is unavailable or locked. Unlock it and try again; no plaintext copy was saved.")
}

impl SecretStore for KeyringStore {
    fn get(&self, reference: &SecretRef) -> Result<SecretString, AiError> {
        Self::entry(reference)?.get_password().map(SecretString::new).map_err(|error| match error {
            keyring::Error::NoEntry => AiError::new(ErrorKind::Auth, "No API key is saved for this connection. Add its key in Assistant settings."),
            _ => unavailable(),
        })
    }

    fn set(&self, reference: &SecretRef, value: &SecretString) -> Result<(), AiError> {
        if value.is_empty() || value.expose().len() > craft_ai_core::draft::MAX_KEY_BYTES {
            return Err(AiError::invalid("The API key is empty or too long."));
        }
        Self::entry(reference)?.set_password(value.expose()).map_err(|_| unavailable())
    }

    fn delete(&self, reference: &SecretRef) -> Result<(), AiError> {
        match Self::entry(reference)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(unavailable()),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemorySecrets(Mutex<BTreeMap<SecretRef, SecretString>>);

    impl SecretStore for MemorySecrets {
        fn get(&self, key: &SecretRef) -> Result<SecretString, AiError> {
            self.0.lock().map_err(|_| unavailable())?.get(key).cloned().ok_or_else(unavailable)
        }
        fn set(&self, key: &SecretRef, value: &SecretString) -> Result<(), AiError> {
            self.0.lock().map_err(|_| unavailable())?.insert(key.clone(), value.clone());
            Ok(())
        }
        fn delete(&self, key: &SecretRef) -> Result<(), AiError> {
            self.0.lock().map_err(|_| unavailable())?.remove(key);
            Ok(())
        }
    }

    #[test]
    fn secrets_are_references_not_serializable_settings() {
        let store = MemorySecrets::default();
        let key = SecretRef::new("test-key").unwrap();
        let value = SecretString::new("synthetic-secret");
        store.set(&key, &value).unwrap();
        assert_eq!(store.get(&key).unwrap().expose(), "synthetic-secret");
        assert!(!format!("{value:?}").contains("synthetic-secret"));
        store.delete(&key).unwrap();
        assert!(store.get(&key).is_err());
    }

    #[test]
    #[ignore = "requires an available OS credential store; writes and removes only a synthetic test entry"]
    fn platform_store_round_trip() {
        let store = KeyringStore;
        let key = SecretRef::new(format!("synthetic-acceptance-{}", std::process::id())).unwrap();
        let value = SecretString::new("craft-assistant-synthetic-acceptance-only");
        store.set(&key, &value).unwrap();
        let read = store.get(&key);
        let deleted = store.delete(&key);
        assert_eq!(read.unwrap().expose(), value.expose());
        deleted.unwrap();
        assert!(store.get(&key).is_err());
    }
}
