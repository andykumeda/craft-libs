//! Shared component location, release source and operational limits.
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use craft_assistant_bridge::Limits;
use craft_assistant_install::{Config, HostCompatibility, Store};
use serde::{Deserialize, Serialize};

const CONFIG_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    pub repo: String,
    pub asset_name: String,
}

impl SourceConfig {
    pub fn validate(&self) -> Result<(), String> {
        let parts: Vec<_> = self.repo.split('/').collect();
        if parts.len() != 2
            || parts.iter().any(|part| {
                part.is_empty()
                    || part.len() > 100
                    || matches!(*part, "." | "..")
                    || !part.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            })
        {
            return Err("Configure the assistant release repository as owner/repository.".into());
        }
        if self.asset_name.is_empty()
            || self.asset_name.len() > 200
            || !self.asset_name.ends_with(".tar.gz")
            || !self.asset_name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err("Configure the exact assistant .tar.gz release asset filename.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentConfig {
    pub schema_version: u32,
    pub installation: Config,
    pub source: SourceConfig,
    #[serde(default = "default_bridge_limits")]
    pub bridge: Limits,
    #[serde(default = "default_startup_timeout_secs")]
    pub startup_timeout_secs: u64,
}

/// The single default source for both production configuration and explicit development launches.
pub fn default_bridge_limits() -> Limits {
    Limits { max_message_bytes: 16 * 1024 * 1024, max_texture_pixels: 4 * 1024 * 1024, max_vertices: 500_000, max_events: 256 }
}

pub fn default_startup_timeout_secs() -> u64 {
    15
}

pub fn compatibility() -> HostCompatibility {
    HostCompatibility { protocol: craft_assistant_bridge::PROTOCOL, settings: craft_ai_core::store::SCHEMA_VERSION, history: 1 }
}

impl ComponentConfig {
    /// `root` is the AI shared-store root, honoring `CRAFT_AI_CONFIG_DIR` in the host.
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join("component.json");
        let file = File::open(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                format!("Configure the shared assistant in {} before installing or updating it.", path.display())
            } else {
                format!("Cannot read the shared assistant configuration: {error}")
            }
        })?;
        let mut bytes = Vec::new();
        file.take(MAX_CONFIG_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Cannot read assistant configuration: {error}"))?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_CONFIG_BYTES {
            return Err("The shared assistant configuration exceeds 64 KiB; it was left untouched.".into());
        }
        let config: Self = serde_json::from_slice(&bytes)
            .map_err(|error| format!("The shared assistant configuration is invalid; it was left untouched: {error}"))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != CONFIG_VERSION {
            return Err(format!(
                "Unsupported assistant component configuration version {}; this app supports {CONFIG_VERSION}. Update the app.",
                self.schema_version
            ));
        }
        Store::new(self.installation.clone())?;
        self.source.validate()?;
        self.bridge.validate()?;
        if !(1..=300).contains(&self.startup_timeout_secs) {
            return Err("Assistant startup_timeout_secs must be between 1 and 300 seconds.".into());
        }
        Ok(())
    }

    pub fn startup_timeout(&self) -> Duration {
        Duration::from_secs(self.startup_timeout_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn config(root: &Path) -> Value {
        json!({
            "schema_version":1,
            "installation":{"root":root,"publisher_team_id":"TESTTEAM","signing_identifier":"org.craft.assistant.test"},
            "source":{"repo":"example/assistant","asset_name":"craft-assistant-macos-arm64.tar.gz"}
        })
    }

    #[test]
    fn missing_corrupt_and_newer_configuration_never_rewrites_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("component.json");
        assert!(ComponentConfig::load(temp.path()).unwrap_err().contains("Configure"));
        std::fs::write(&path, b"{").unwrap();
        assert!(ComponentConfig::load(temp.path()).unwrap_err().contains("left untouched"));
        assert_eq!(std::fs::read(&path).unwrap(), b"{");
        let mut value = config(&temp.path().join("installed"));
        value["schema_version"] = json!(999);
        let bytes = serde_json::to_vec(&value).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert!(ComponentConfig::load(temp.path()).unwrap_err().contains("Update the app"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn defaults_and_operational_overrides_have_one_source() {
        let temp = tempfile::tempdir().unwrap();
        let mut value = config(&temp.path().join("installed"));
        let config: ComponentConfig = serde_json::from_value(value.clone()).unwrap();
        config.validate().unwrap();
        assert_eq!(config.bridge.max_message_bytes, default_bridge_limits().max_message_bytes);
        assert_eq!(config.startup_timeout(), Duration::from_secs(default_startup_timeout_secs()));
        value["startup_timeout_secs"] = json!(30);
        value["bridge"] = serde_json::to_value(default_bridge_limits()).unwrap();
        value["bridge"]["max_events"] = json!(32);
        let config: ComponentConfig = serde_json::from_value(value).unwrap();
        config.validate().unwrap();
        assert_eq!(config.bridge.max_events, 32);
        assert_eq!(config.startup_timeout(), Duration::from_secs(30));
    }

    #[test]
    fn invalid_sources_limits_timeouts_and_unknown_fields_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let original = config(&temp.path().join("installed"));
        for repo in ["", "https://github.com/x/y", "x/../y", "x/..", "x/y?token=private"] {
            let mut value = original.clone();
            value["source"]["repo"] = json!(repo);
            assert!(serde_json::from_value::<ComponentConfig>(value).unwrap().validate().is_err());
        }
        for seconds in [0, 301, u64::MAX] {
            let mut value = original.clone();
            value["startup_timeout_secs"] = json!(seconds);
            assert!(serde_json::from_value::<ComponentConfig>(value).unwrap().validate().is_err());
        }
        let mut value = original.clone();
        value["bridge"] = serde_json::to_value(default_bridge_limits()).unwrap();
        value["bridge"]["max_vertices"] = json!(0);
        assert!(serde_json::from_value::<ComponentConfig>(value).unwrap().validate().is_err());
        let mut value = original;
        value["provider_fallback"] = json!("another-service");
        assert!(serde_json::from_value::<ComponentConfig>(value).is_err());
    }
}
