//! Explicitly requested assistant release checks and approved installation, on worker threads.
use std::path::Path;

use craft_assistant_install::{Installed, Package, Store};
use craft_update::protocol::LatestRelease;
use craft_update::release::{Asset, Release};
use craft_update::source::{GitHubSource, ReleaseSource};
use craft_update::{Http, Stop};
use semver::Version;

use crate::config::{ComponentConfig, compatibility};

#[derive(Clone, Debug)]
pub struct Candidate {
    pub version: Version,
    pub asset: Asset,
}

/// This only checks metadata. No download or install occurs until `install` is explicitly called.
pub fn check(config: &ComponentConfig, current: Option<&str>) -> Result<Option<Candidate>, String> {
    config.validate()?;
    let http = Http::new(concat!("CraftAssistant/", env!("CARGO_PKG_VERSION")))?;
    // A fresh source avoids silently offering stale cached metadata after a failed requested check.
    let temporary = tempfile::tempdir().map_err(|error| format!("Cannot stage an assistant update check: {error}"))?;
    let source = GitHubSource::new(http.clone(), temporary.path().join("release-cache.json"));
    let release = match source.latest(&config.source.repo, false).latest {
        LatestRelease::Found(release) => release,
        LatestRelease::None => return Ok(None),
        LatestRelease::Failed(error) => return Err(format!("Cannot check assistant updates: {error}")),
    };
    let mut candidate = select(config, &release, current)?;
    if let Some(candidate) = candidate.as_mut()
        && candidate.asset.sha256.is_none()
    {
        let sums = release.sums_asset().ok_or("The assistant release has no SHA-256 checksum for the configured asset.")?;
        validate_url(config, &sums.url)?;
        let limit = usize::try_from(config.installation.limits.metadata_bytes).map_err(|_| "Assistant metadata limit does not fit this platform.")?;
        let bytes = http.get_bytes(&sums.url, limit)?.ok_or("The assistant release checksum file is missing.")?;
        let text = std::str::from_utf8(&bytes).map_err(|_| "The assistant release checksum file is not UTF-8.")?;
        candidate.asset.sha256 =
            craft_update::release::parse_sha256sums(text).into_iter().find(|(_, name)| name == &candidate.asset.name).map(|(hash, _)| hash);
    }
    if let Some(candidate) = &candidate {
        validate_candidate(config, candidate)?;
    }
    Ok(candidate)
}

/// Call only after approval and while the app's panel has no active turn. Existing helper processes
/// keep using their immutable versions; the host resolves activation at its next eligible launch.
pub fn install(config: &ComponentConfig, candidate: &Candidate) -> Result<Installed, String> {
    config.validate()?;
    validate_candidate(config, candidate)?;
    let temporary = tempfile::tempdir().map_err(|error| format!("Cannot stage the assistant download: {error}"))?;
    let archive = temporary.path().join("assistant.tar.gz");
    let checksum = candidate.asset.sha256.as_deref().ok_or("The assistant release has no checksum.")?;
    let http = Http::new(concat!("CraftAssistant/", env!("CARGO_PKG_VERSION")))?;
    http.download(&candidate.asset.url, &archive, candidate.asset.size, checksum, &Stop::default(), &mut |_| {})?;
    install_archive(config, candidate, &archive)
}

fn install_archive(config: &ComponentConfig, candidate: &Candidate, archive: &Path) -> Result<Installed, String> {
    let sha256 = candidate.asset.sha256.clone().ok_or("The assistant release has no checksum.")?;
    Store::new(config.installation.clone())?.install(archive, &Package { version: candidate.version.clone(), sha256 }, &compatibility())
}

fn select(config: &ComponentConfig, release: &Release, current: Option<&str>) -> Result<Option<Candidate>, String> {
    if release.prerelease {
        return Err("The configured assistant source returned a prerelease for the stable channel.".into());
    }
    let version = release.version.as_ref().ok_or("The assistant release has no valid version.")?;
    if !version.pre.is_empty() {
        return Err("The configured assistant release is not a stable version.".into());
    }
    if let Some(current) = current {
        let current = Version::parse(current).map_err(|_| "The installed assistant reported an invalid version.")?;
        if version.cmp_precedence(&current).is_le() {
            return Ok(None);
        }
    }
    let mut assets = release.assets.iter().filter(|asset| asset.name == config.source.asset_name);
    let asset = assets.next().ok_or("The assistant release does not contain the exact configured platform asset.")?;
    if assets.next().is_some() {
        return Err("The assistant release repeats the configured asset; it was refused.".into());
    }
    let candidate = Candidate { version: version.clone(), asset: asset.clone() };
    validate_asset(config, &candidate.asset)?;
    Ok(Some(candidate))
}

fn validate_url(config: &ComponentConfig, url: &str) -> Result<(), String> {
    let prefix = craft_update::release::download_prefix(&config.source.repo);
    if !craft_update::release::download_url_under(url, &prefix) {
        return Err("The assistant asset URL is outside the configured repository's release downloads.".into());
    }
    Ok(())
}

fn validate_asset(config: &ComponentConfig, asset: &Asset) -> Result<(), String> {
    if asset.name != config.source.asset_name {
        return Err("The assistant asset differs from the configured platform filename.".into());
    }
    if asset.size == 0 || asset.size > config.installation.limits.archive_bytes {
        return Err("The assistant asset must declare a positive size within the configured download limit.".into());
    }
    validate_url(config, &asset.url)?;
    if asset.url.rsplit('/').next() != Some(asset.name.as_str()) {
        return Err("The assistant download URL does not name the configured platform asset.".into());
    }
    Ok(())
}

fn validate_candidate(config: &ComponentConfig, candidate: &Candidate) -> Result<(), String> {
    validate_asset(config, &candidate.asset)?;
    if !candidate.version.pre.is_empty() {
        return Err("Only stable assistant releases are accepted.".into());
    }
    if !candidate.asset.sha256.as_deref().is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())) {
        return Err("The assistant release has no valid SHA-256 checksum for the configured asset.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{SourceConfig, default_bridge_limits};

    fn fixture() -> (ComponentConfig, Release) {
        let config = ComponentConfig {
            schema_version: 1,
            installation: craft_assistant_install::Config {
                root: std::env::temp_dir().join("synthetic-assistant-never-created"),
                publisher_team_id: "TESTTEAM".into(),
                signing_identifier: "org.craft.assistant.test".into(),
                local_certificate_sha1: None,
                limits: craft_assistant_install::Limits::default(),
            },
            source: SourceConfig { repo: "example/assistant".into(), asset_name: "craft-assistant-macos-arm64.tar.gz".into() },
            bridge: default_bridge_limits(),
            startup_timeout_secs: 15,
        };
        let release = Release {
            tag: "v1.1.0".into(),
            version: Some(Version::parse("1.1.0").unwrap()),
            assets: vec![Asset {
                name: config.source.asset_name.clone(),
                size: 100,
                url: format!("https://github.com/example/assistant/releases/download/v1.1.0/{}", config.source.asset_name),
                sha256: Some("a".repeat(64)),
            }],
            ..Release::default()
        };
        (config, release)
    }

    #[test]
    fn stable_exact_platform_asset_and_newer_version_are_required() {
        let (config, mut release) = fixture();
        let candidate = select(&config, &release, Some("1.0.0")).unwrap().unwrap();
        validate_candidate(&config, &candidate).unwrap();
        assert!(select(&config, &release, Some("1.1.0")).unwrap().is_none());
        assert!(select(&config, &release, Some("1.2.0")).unwrap().is_none());
        release.assets[0].name = "craft-assistant-windows.tar.gz".into();
        assert!(select(&config, &release, None).unwrap_err().contains("exact configured"));
        release.prerelease = true;
        assert!(select(&config, &release, None).unwrap_err().contains("prerelease"));
    }

    #[test]
    fn wrong_repository_missing_checksum_and_unbounded_size_are_refused() {
        let (config, release) = fixture();
        let mut candidate = select(&config, &release, None).unwrap().unwrap();
        candidate.asset.url = "https://github.com/other/assistant/releases/download/v1.1.0/file.tar.gz".into();
        assert!(validate_candidate(&config, &candidate).unwrap_err().contains("outside"));
        candidate.asset.url = "https://github.com/example/assistant/releases/download/v1.1.0/other-platform.tar.gz".into();
        assert!(validate_candidate(&config, &candidate).unwrap_err().contains("platform asset"));
        candidate.asset = release.assets[0].clone();
        candidate.asset.sha256 = None;
        assert!(validate_candidate(&config, &candidate).unwrap_err().contains("checksum"));
        candidate.asset = release.assets[0].clone();
        for size in [0, config.installation.limits.archive_bytes.saturating_add(1)] {
            candidate.asset.size = size;
            assert!(validate_candidate(&config, &candidate).unwrap_err().contains("download limit"));
        }
    }

    #[test]
    fn duplicate_asset_does_not_choose_an_arbitrary_download() {
        let (config, mut release) = fixture();
        release.assets.push(release.assets[0].clone());
        assert!(select(&config, &release, None).unwrap_err().contains("repeats"));
    }
}
