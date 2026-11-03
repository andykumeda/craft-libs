//! Explicit release policy for assistant-enabled app builds. No upstream fallback.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
#[cfg(feature = "egui")]
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{App, Http, Release};

const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const MANIFEST_ASSET: &str = "craft-assistant-app.json";

/// One app's approved repository, publisher and embedded assistant bridge contract.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub repo: String,
    pub macos_team_id: String,
    pub bridge_protocol: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sources {
    schema_version: u32,
    apps: BTreeMap<String, Policy>,
}

/// Release-side declaration. The signed app bundle independently proves these values at install.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppManifest {
    pub schema_version: u32,
    pub app_id: String,
    pub release_tag: String,
    pub repository: String,
    pub bridge_protocol: u32,
}

impl Policy {
    /// Loads only the explicitly configured app. Missing and invalid policies never select upstream.
    pub fn load(root: &Path, app: &App, bridge_protocol: u32) -> Result<Self, String> {
        let path = root.join("sources.json");
        let file = std::fs::File::open(&path).map_err(|e| format!("Configure assistant-enabled app updates in {} ({e})", path.display()))?;
        let mut bytes = Vec::new();
        file.take(MAX_CONFIG_BYTES.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| format!("Could not read update sources ({e})"))?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err("Update sources exceed 64 KiB; the configuration was left untouched.".into());
        }
        Self::parse(&bytes, app, bridge_protocol)
    }

    fn parse(bytes: &[u8], app: &App, bridge_protocol: u32) -> Result<Self, String> {
        let sources: Sources = serde_json::from_slice(bytes).map_err(|e| format!("Invalid assistant update sources ({e})"))?;
        if sources.schema_version != 1 {
            return Err("Unsupported assistant update source schema; the configuration was left untouched.".into());
        }
        let policy = sources.apps.get(app.id).ok_or_else(|| format!("No assistant-enabled update source is configured for {}.", app.name))?.clone();
        policy.validate(bridge_protocol)?;
        Ok(policy)
    }

    pub(crate) fn validate(&self, bridge_protocol: u32) -> Result<(), String> {
        let mut parts = self.repo.split('/');
        let component = |part: &str| {
            !part.is_empty()
                && part.len() <= 100
                && part != "."
                && part != ".."
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        };
        if !parts.next().is_some_and(component) || !parts.next().is_some_and(component) || parts.next().is_some() {
            return Err("The update repository must be a GitHub owner/repository, without a URL or path.".into());
        }
        if self.macos_team_id.len() != 10 || !self.macos_team_id.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()) {
            return Err("Configure the custom release publisher's ten-character Apple Developer team ID.".into());
        }
        if bridge_protocol == 0 || self.bridge_protocol != bridge_protocol {
            return Err(format!(
                "This app requires assistant bridge protocol {bridge_protocol}; its update source declares {}.",
                self.bridge_protocol
            ));
        }
        Ok(())
    }

    /// Checks compatible stable releases only in the approved repository.
    pub fn check(&self, app: &App, http: &Http, current: &str) -> Result<Option<Release>, String> {
        self.validate(self.bridge_protocol)?;
        let url = format!("https://api.github.com/repos/{}/releases?per_page=30", self.repo);
        let text = match http.get_api(&url, None)? {
            crate::http::Fetched::Body { text, .. } => text,
            crate::http::Fetched::NotFound => {
                return Err(format!("The configured update repository {} is unavailable; no alternate source was used.", self.repo));
            }
            crate::http::Fetched::RateLimited { .. } => return Err("GitHub's request limit was reached; try the configured source later.".into()),
            crate::http::Fetched::NotModified => return Err("The configured update source returned an unexpected unchanged response.".into()),
        };
        let Some(release) = crate::release::parse_github_release_list(&text, &self.repo, false)? else {
            return Ok(None);
        };
        let (Some(want), Some(have)) = (release.version.as_ref(), crate::version::parse(current)) else {
            return Err("The configured app or release version is not a supported semantic version.".into());
        };
        if !crate::version::is_newer(want, &have) {
            return Ok(None);
        }
        self.check_release(app, http, &release)?;
        Ok(Some(release))
    }

    /// Rechecks the exact offered release before installation, without fetching a different release.
    pub fn check_release(&self, app: &App, http: &Http, release: &Release) -> Result<(), String> {
        self.validate(self.bridge_protocol)?;
        let prefix = format!("https://github.com/{}/releases/download/", self.repo);
        let manifest = release
            .assets
            .iter()
            .find(|asset| asset.name == MANIFEST_ASSET)
            .ok_or("This release does not declare an assistant bridge; it cannot replace this app.")?;
        if !manifest.url.starts_with(&prefix) {
            return Err("The assistant release manifest is outside the configured repository.".into());
        }
        let bytes = http.get_bytes(&manifest.url, MAX_CONFIG_BYTES as usize)?.ok_or("The assistant app release manifest is missing.")?;
        self.check_manifest(app, release, &bytes)
    }

    fn check_manifest(&self, app: &App, release: &Release, bytes: &[u8]) -> Result<(), String> {
        let manifest: AppManifest = serde_json::from_slice(bytes).map_err(|e| format!("Invalid assistant app release manifest ({e})"))?;
        if manifest.schema_version != 1
            || manifest.app_id != app.id
            || manifest.release_tag != release.tag
            || manifest.repository != self.repo
            || manifest.bridge_protocol != self.bridge_protocol
        {
            return Err("This release's assistant bridge, app identity or repository is incompatible; it was not installed.".into());
        }
        Ok(())
    }

    /// Checks metadata covered by the bundle's code signature, before replacement.
    #[cfg(target_os = "macos")]
    pub(crate) fn check_bundle(&self, bundle: &Path) -> Result<(), String> {
        let file = std::fs::File::open(bundle.join("Contents/Info.plist")).map_err(|e| format!("Could not read the signed app metadata ({e})"))?;
        let mut bytes = Vec::new();
        file.take(MAX_CONFIG_BYTES.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| format!("Could not read the signed app metadata ({e})"))?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err("The signed app metadata exceeds 64 KiB; the app was not installed.".into());
        }
        let info = plist::Value::from_reader(std::io::Cursor::new(bytes)).map_err(|e| format!("Could not parse the signed app metadata ({e})"))?;
        let dict = info.as_dictionary().ok_or("The signed app metadata is not a dictionary.")?;
        let protocol = dict.get("CraftAssistantBridgeProtocol").and_then(plist::Value::as_unsigned_integer);
        let repo = dict.get("CraftUpdateRepository").and_then(plist::Value::as_string);
        if protocol != Some(u64::from(self.bridge_protocol)) || repo != Some(self.repo.as_str()) {
            return Err("The signed app bundle does not contain the configured assistant bridge and update source; it was not installed.".into());
        }
        Ok(())
    }
}

/// Creates an app updater which can never use the upstream release source.
#[cfg(feature = "egui")]
pub fn updater(app: &'static App, version: impl Into<String>, root: Result<PathBuf, String>, bridge_protocol: u32) -> crate::updater::Updater {
    let policy = root.and_then(|root| Policy::load(&root, app, bridge_protocol));
    crate::updater::Updater::new(app, version).with_custom_policy(policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> Policy {
        Policy { repo: "my-account/pdfcraft".into(), macos_team_id: "ABC1234567".into(), bridge_protocol: 1 }
    }

    #[test]
    fn explicit_sources_are_required_and_validate_identity_and_protocol() {
        let app = &crate::suite::PDFCRAFT;
        let missing = tempfile::tempdir().unwrap();
        assert!(Policy::load(missing.path(), app, 1).unwrap_err().contains("sources.json"));
        assert!(Policy::parse(br#"{"schema_version":1,"apps":{}}"#, app, 1).is_err());
        let json = serde_json::json!({"schema_version":1,"apps":{"pdfcraft":policy()}});
        assert_eq!(Policy::parse(&serde_json::to_vec(&json).unwrap(), app, 1).unwrap(), policy());
        assert!(Policy::parse(&serde_json::to_vec(&json).unwrap(), app, 2).is_err());
        assert!(Policy { repo: "https://github.com/a/b".into(), ..policy() }.validate(1).is_err());
        assert!(Policy { macos_team_id: String::new(), ..policy() }.validate(1).is_err());
        assert!(Policy { repo: "../pdfcraft".into(), ..policy() }.validate(1).is_err());
    }

    #[test]
    fn oversized_source_file_is_refused_without_modifying_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sources.json");
        let bytes = vec![b' '; MAX_CONFIG_BYTES as usize + 1];
        std::fs::write(&path, &bytes).unwrap();
        assert!(Policy::load(dir.path(), &crate::suite::PDFCRAFT, 1).unwrap_err().contains("64 KiB"));
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn ordinary_releases_are_refused_before_network_or_installation() {
        let http = Http::new("CraftUpdate-Synthetic-Test").unwrap();
        assert!(policy().check_release(&crate::suite::PDFCRAFT, &http, &Release::default()).unwrap_err().contains("does not declare"));
        let mut release = Release::default();
        release.assets.push(crate::release::Asset {
            name: MANIFEST_ASSET.into(),
            url: "https://github.com/storytold/pdfcraft/releases/download/v1/craft-assistant-app.json".into(),
            size: 1,
            sha256: None,
        });
        assert!(policy().check_release(&crate::suite::PDFCRAFT, &http, &release).unwrap_err().contains("outside"));
    }

    #[test]
    fn release_manifest_is_bound_to_app_tag_source_and_bridge() {
        let app = &crate::suite::PDFCRAFT;
        let release = Release { tag: "v1.2.3".into(), ..Default::default() };
        let manifest =
            AppManifest { schema_version: 1, app_id: app.id.into(), release_tag: release.tag.clone(), repository: policy().repo, bridge_protocol: 1 };
        let bytes = serde_json::to_vec(&manifest).unwrap();
        policy().check_manifest(app, &release, &bytes).unwrap();
        assert!(policy().check_manifest(&crate::suite::WORDCRAFT, &release, &bytes).is_err());
        assert!(policy().check_manifest(app, &Release { tag: "v9.9.9".into(), ..release }, &bytes).is_err());
        assert!(
            Policy { bridge_protocol: 2, ..policy() }.check_manifest(app, &Release { tag: "v1.2.3".into(), ..Default::default() }, &bytes).is_err()
        );
        assert!(policy().check_manifest(app, &Release::default(), b"not json").is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unsigned_metadata_declarations_do_not_substitute_for_signed_bundle_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let contents = dir.path().join("Contents");
        std::fs::create_dir(&contents).unwrap();
        assert!(policy().check_bundle(dir.path()).is_err());
        let mut dict = plist::Dictionary::new();
        dict.insert("CraftAssistantBridgeProtocol".into(), plist::Value::Integer(1.into()));
        dict.insert("CraftUpdateRepository".into(), plist::Value::String(policy().repo));
        plist::Value::Dictionary(dict.clone()).to_file_xml(contents.join("Info.plist")).unwrap();
        policy().check_bundle(dir.path()).unwrap();
        dict.insert("CraftAssistantBridgeProtocol".into(), plist::Value::Integer(2.into()));
        plist::Value::Dictionary(dict).to_file_xml(contents.join("Info.plist")).unwrap();
        assert!(policy().check_bundle(dir.path()).is_err());
        assert!(!crate::install::macos::signed_by(dir.path(), &policy().macos_team_id));
    }
}
