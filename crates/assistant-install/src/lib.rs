//! Immutable assistant bundles and atomic activation, independent of any application's bundle.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use craft_atomic::{NewFilePermissions, WriteOptions};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const BUNDLE: &str = "CraftAssistant.app";
pub const EXECUTABLE: &str = "Contents/MacOS/craft-assistant";
pub const MANIFEST: &str = "Contents/Resources/assistant-manifest.json";

/// Inclusive versions of a wire or storage format. Storage writers must preserve all of this range.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    pub min: u32,
    pub max: u32,
}

impl Range {
    fn accepts(&self, version: u32) -> bool {
        self.min > 0 && self.min <= self.max && (self.min..=self.max).contains(&version)
    }
}

/// Lives inside the signed bundle, so the signer authenticates these claims as well as the code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: Version,
    pub protocol: Range,
    pub settings: Range,
    pub history: Range,
}

#[derive(Clone, Copy, Debug)]
pub struct HostCompatibility {
    pub protocol: u32,
    pub settings: u32,
    pub history: u32,
}

impl Manifest {
    fn check(&self, host: &HostCompatibility) -> Result<(), String> {
        for (name, supported, actual) in
            [("protocol", &self.protocol, host.protocol), ("settings", &self.settings, host.settings), ("history", &self.history, host.history)]
        {
            if !supported.accepts(actual) {
                return Err(format!(
                    "Assistant {} requires {name} {}..={}; this app uses {actual}. Install a compatible assistant or update the app.",
                    self.version, supported.min, supported.max
                ));
            }
        }
        Ok(())
    }
}

/// Integrity claim from the selected release. A checksum alone is not publisher authentication.
#[derive(Clone, Debug)]
pub struct Package {
    pub version: Version,
    pub sha256: String,
}

/// One documented default source; callers may override limits through their configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub archive_bytes: u64,
    pub unpacked_bytes: u64,
    pub entries: u32,
    pub metadata_bytes: u64,
    pub verification_timeout_secs: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            archive_bytes: 128 * 1024 * 1024,
            unpacked_bytes: 256 * 1024 * 1024,
            entries: 512,
            metadata_bytes: 64 * 1024,
            verification_timeout_secs: 120,
        }
    }
}

/// Supply the shared data root and your own release signing identity. No publisher fallback exists.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub root: PathBuf,
    pub publisher_team_id: String,
    pub signing_identifier: String,
    #[serde(default)]
    pub limits: Limits,
}

impl Config {
    fn validate(&self) -> Result<(), String> {
        if !self.root.is_absolute() || self.root.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err("Choose an absolute assistant installation directory without '..'.".into());
        }
        if inside_app_bundle(&self.root) {
            return Err("The shared assistant directory must be outside application bundles.".into());
        }
        if self.publisher_team_id.is_empty() || !self.publisher_team_id.bytes().all(|c| c.is_ascii_alphanumeric()) {
            return Err("Configure the assistant publisher's Apple team identifier.".into());
        }
        if self.signing_identifier.is_empty()
            || !self.signing_identifier.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_'))
        {
            return Err("Configure a valid assistant code-signing identifier.".into());
        }
        let limits = &self.limits;
        if limits.archive_bytes == 0
            || limits.unpacked_bytes == 0
            || limits.entries == 0
            || limits.metadata_bytes == 0
            || limits.verification_timeout_secs == 0
        {
            return Err("Assistant installation limits must be positive.".into());
        }
        if limits.archive_bytes > 1024 * 1024 * 1024
            || limits.unpacked_bytes > 2 * 1024 * 1024 * 1024
            || limits.entries > 100_000
            || limits.metadata_bytes > 1024 * 1024
            || limits.verification_timeout_secs > 300
        {
            return Err("Assistant installation limits exceed the supported safety ceilings (1 GiB compressed, 2 GiB unpacked, 100000 entries, 1 MiB metadata, 300 seconds verification).".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Installed {
    pub version: Version,
    pub executable: PathBuf,
    pub bundle: PathBuf,
    pub manifest: Manifest,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Active {
    version: Version,
    previous: Option<Version>,
}

/// Shared by all app instances; the OS lock serializes installation and activation across processes.
pub struct Store {
    config: Config,
}

impl Store {
    pub fn new(config: Config) -> Result<Self, String> {
        config.validate()?;
        Ok(Self { config })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Worker-thread operation, called only after the user approves the selected release.
    pub fn install(&self, archive: &Path, package: &Package, host: &HostCompatibility) -> Result<Installed, String> {
        self.install_checked(archive, package, host, verify_signature)
    }

    /// Resolve the active installation without downloading or contacting Gatekeeper. Incompatibility
    /// is reported, never silently replaced with a different release or provider.
    pub fn resolve(&self, host: &HostCompatibility) -> Result<Installed, String> {
        self.resolve_checked(host, verify_signature)
    }

    /// Explicit rollback/activation; retained bundles and conversation files are never rewritten.
    pub fn activate(&self, version: &Version, host: &HostCompatibility) -> Result<Installed, String> {
        self.activate_checked(version, host, verify_signature)
    }

    pub fn previous_version(&self) -> Result<Option<Version>, String> {
        Ok(self.active()?.and_then(|active| active.previous))
    }

    fn root(&self) -> Result<PathBuf, String> {
        private_dir(&self.config.root)?;
        let root = self.config.root.canonicalize().map_err(|e| format!("Cannot resolve the assistant directory: {e}"))?;
        if inside_app_bundle(&root) {
            return Err("The resolved shared assistant directory is inside an application bundle.".into());
        }
        Ok(root)
    }

    fn lock(&self) -> Result<File, String> {
        let path = self.root()?.join("install.lock");
        reject_symlink(&path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path).map_err(|e| format!("Cannot open the assistant installation lock: {e}"))?;
        file.lock().map_err(|e| format!("Cannot lock the assistant installation: {e}"))?;
        Ok(file)
    }

    fn active(&self) -> Result<Option<Active>, String> {
        let path = self.root()?.join("active.json");
        match read_bounded(&path, self.config.limits.metadata_bytes) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map(Some).map_err(|e| format!("Assistant activation data is invalid; it was left untouched: {e}"))
            }
            Err(ReadError::Missing) => Ok(None),
            Err(ReadError::Other(error)) => Err(error),
        }
    }

    fn installed(&self, version: &Version, host: &HostCompatibility, verify: Verifier) -> Result<Installed, String> {
        let versions = self.root()?.join("versions");
        reject_symlink(&versions)?;
        let directory = versions.join(version.to_string());
        reject_symlink(&directory)?;
        let bundle = directory.join(BUNDLE);
        reject_symlink(&bundle)?;
        verify(&bundle, &self.config, false)?;
        let manifest = self.manifest(&bundle)?;
        if &manifest.version != version {
            return Err("The installed assistant does not match its version directory; reinstall it.".into());
        }
        manifest.check(host)?;
        let executable = bundle_file(&bundle, EXECUTABLE)?;
        if !executable.is_file() {
            return Err("The assistant executable is missing; reinstall the assistant.".into());
        }
        Ok(Installed { version: manifest.version.clone(), executable, bundle, manifest })
    }

    fn manifest(&self, bundle: &Path) -> Result<Manifest, String> {
        let bytes = read_bounded(&bundle_file(bundle, MANIFEST)?, self.config.limits.metadata_bytes).map_err(|e| e.message())?;
        serde_json::from_slice(&bytes).map_err(|e| format!("The signed assistant manifest is invalid: {e}"))
    }

    fn commit_active(&self, version: &Version) -> Result<(), String> {
        let active = self.active()?;
        if active.as_ref().is_some_and(|active| &active.version == version) {
            return Ok(());
        }
        let pointer = Active { version: version.clone(), previous: active.map(|active| active.version) };
        let bytes = serde_json::to_vec(&pointer).map_err(|e| format!("Cannot encode assistant activation: {e}"))?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > self.config.limits.metadata_bytes {
            return Err("Assistant activation exceeds the configured metadata limit.".into());
        }
        let path = self.root()?.join("active.json");
        reject_symlink(&path)?;
        WriteOptions::default()
            .with_new_file_permissions(NewFilePermissions::Private)
            .atomic_write(&path, &bytes)
            .map_err(|e| format!("Cannot activate the assistant; the previous activation was preserved: {e}"))
    }

    fn install_checked(&self, archive: &Path, package: &Package, host: &HostCompatibility, verify: Verifier) -> Result<Installed, String> {
        let _lock = self.lock()?;
        let root = self.root()?;
        let versions = root.join("versions");
        private_dir(&versions)?;
        let destination = versions.join(package.version.to_string());
        if destination.try_exists().map_err(|e| format!("Cannot inspect the assistant installation: {e}"))? {
            return Err(
                "That assistant version is already installed. Activate it explicitly or select a newer release; installed versions are immutable."
                    .into(),
            );
        }
        if self.active()?.is_some_and(|active| package.version.cmp_precedence(&active.version).is_le()) {
            return Err("An equal or newer assistant is already active. Refresh the update check; use explicit activation to roll back.".into());
        }
        let archive = checked_archive(archive, &package.sha256, self.config.limits.archive_bytes)?;
        let staging =
            tempfile::Builder::new().prefix(".assistant-stage-").tempdir_in(&root).map_err(|e| format!("Cannot stage the assistant: {e}"))?;
        unpack(archive, staging.path(), &self.config.limits)?;
        let bundle = staging.path().join(BUNDLE);
        verify(&bundle, &self.config, true)?;
        let manifest = self.manifest(&bundle)?;
        if manifest.version != package.version {
            return Err("The signed assistant version does not match the selected release.".into());
        }
        manifest.check(host)?;
        if !bundle_file(&bundle, EXECUTABLE)?.is_file() {
            return Err("The assistant package has no executable.".into());
        }
        // Complete immutable content first; a failed pointer write merely leaves an inactive version.
        std::fs::rename(staging.path(), &destination).map_err(|e| format!("Cannot finish the assistant installation: {e}"))?;
        let _ = File::open(&versions).and_then(|directory| craft_atomic::sync_file(&directory));
        self.commit_active(&package.version)?;
        Ok(Installed {
            version: manifest.version.clone(),
            executable: destination.join(BUNDLE).join(EXECUTABLE),
            bundle: destination.join(BUNDLE),
            manifest,
        })
    }

    fn resolve_checked(&self, host: &HostCompatibility, verify: Verifier) -> Result<Installed, String> {
        let active = self.active()?.ok_or("The shared assistant is not installed. Choose Install assistant in the panel.")?;
        self.installed(&active.version, host, verify)
    }

    fn activate_checked(&self, version: &Version, host: &HostCompatibility, verify: Verifier) -> Result<Installed, String> {
        let _lock = self.lock()?;
        let installed = self.installed(version, host, verify)?;
        self.commit_active(version)?;
        Ok(installed)
    }
}

type Verifier = fn(&Path, &Config, bool) -> Result<(), String>;

fn inside_app_bundle(path: &Path) -> bool {
    path.components().any(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase().ends_with(".app"))
}

#[cfg(target_os = "macos")]
fn verify_signature(bundle: &Path, config: &Config, assess: bool) -> Result<(), String> {
    use std::time::Duration;
    let requirement = format!(
        "=anchor apple generic and certificate leaf[subject.OU] = \"{}\" and identifier \"{}\"",
        config.publisher_team_id, config.signing_identifier
    );
    let mut command = std::process::Command::new("/usr/bin/codesign");
    command.args(["--verify", "--strict", "--deep", "-R", &requirement]).arg(bundle);
    craft_update::process::run_command(command, "codesign", Duration::from_secs(config.limits.verification_timeout_secs))
        .map_err(|_| "The assistant signature is invalid or belongs to a different publisher. The current installation was preserved.".to_string())?;
    if assess {
        let mut command = std::process::Command::new("/usr/sbin/spctl");
        command.args(["--assess", "--type", "execute"]).arg(bundle);
        craft_update::process::run_command(command, "spctl", Duration::from_secs(config.limits.verification_timeout_secs)).map_err(|_| {
            "macOS did not approve this assistant package. Check its notarization and try again; the current installation was preserved.".to_string()
        })?;
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn verify_signature(_: &Path, _: &Config, _: bool) -> Result<(), String> {
    Err("Shared assistant installation is currently supported only on macOS.".into())
}

fn private_dir(path: &Path) -> Result<(), String> {
    reject_symlink(path)?;
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|e| format!("Cannot create assistant directory {}: {e}", path.display()))
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!("The assistant installation path is a symbolic link: {}", path.display())),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Cannot inspect assistant path {}: {error}", path.display())),
    }
}

fn bundle_file(bundle: &Path, relative: &str) -> Result<PathBuf, String> {
    let mut path = bundle.to_path_buf();
    for component in Path::new(relative).components() {
        path.push(component);
        reject_symlink(&path)?;
    }
    Ok(path)
}

enum ReadError {
    Missing,
    Other(String),
}

impl ReadError {
    fn message(self) -> String {
        match self {
            Self::Missing => "An assistant installation file is missing; reinstall the assistant.".into(),
            Self::Other(error) => error,
        }
    }
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, ReadError> {
    reject_symlink(path).map_err(ReadError::Other)?;
    let file = File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ReadError::Missing
        } else {
            ReadError::Other(format!("Cannot read assistant installation: {error}"))
        }
    })?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| ReadError::Other(format!("Cannot read assistant installation: {e}")))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(ReadError::Other("Assistant installation metadata exceeds its configured limit.".into()));
    }
    Ok(bytes)
}

fn checked_archive(path: &Path, expected: &str, limit: u64) -> Result<File, String> {
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("The selected assistant release has no valid SHA-256 checksum.".into());
    }
    if !std::fs::metadata(path).map_err(|e| format!("Cannot inspect the assistant package: {e}"))?.is_file() {
        return Err("Choose a regular assistant package file.".into());
    }
    let mut file = File::open(path).map_err(|e| format!("Cannot read the assistant package: {e}"))?;
    let mut hasher = Sha256::new();
    let count = std::io::copy(&mut Read::by_ref(&mut file).take(limit.saturating_add(1)), &mut hasher)
        .map_err(|e| format!("Cannot checksum the assistant package: {e}"))?;
    if count > limit {
        return Err("The assistant package exceeds the configured download limit.".into());
    }
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        return Err("The assistant package checksum does not match the release. Download it again; the current installation was preserved.".into());
    }
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0)).map_err(|e| format!("Cannot reopen the assistant package: {e}"))?;
    Ok(file)
}

fn unpack(file: File, destination: &Path, limits: &Limits) -> Result<(), String> {
    let expanded_limit = limits.unpacked_bytes.saturating_add(u64::from(limits.entries).saturating_mul(1024)).saturating_add(1024);
    let gzip = flate2::read::GzDecoder::new(file).take(expanded_limit);
    let mut archive = tar::Archive::new(gzip);
    let mut entries = 0u32;
    let mut bytes = 0u64;
    let mut seen = BTreeSet::new();
    for entry in archive.entries().map_err(|e| format!("Cannot read the assistant archive: {e}"))? {
        let mut entry = entry.map_err(|e| format!("The assistant archive is damaged: {e}"))?;
        entries = entries.checked_add(1).ok_or("Too many assistant archive entries.")?;
        if entries > limits.entries {
            return Err("The assistant archive exceeds its configured entry limit.".into());
        }
        let path = entry.path().map_err(|e| format!("Invalid assistant archive path: {e}"))?.into_owned();
        if path.components().any(|c| !matches!(c, Component::Normal(_))) || !path.starts_with(BUNDLE) || !seen.insert(path.clone()) {
            return Err("The assistant archive contains a duplicate or escaping path.".into());
        }
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            return Err("The assistant archive contains links or unsupported entries.".into());
        }
        bytes = bytes.checked_add(entry.size()).ok_or("The assistant archive is too large.")?;
        if bytes > limits.unpacked_bytes {
            return Err("The assistant archive exceeds its configured unpacked size limit.".into());
        }
        let output = destination.join(&path);
        if kind.is_dir() {
            private_dir(&output)?;
        } else {
            if let Some(parent) = output.parent() {
                private_dir(parent)?;
            }
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                let executable = path == Path::new(BUNDLE).join(EXECUTABLE);
                options.mode(if executable { 0o700 } else { 0o600 });
            }
            let mut file = options.open(output).map_err(|e| format!("Cannot unpack the assistant: {e}"))?;
            let count = std::io::copy(&mut entry, &mut file).map_err(|e| format!("Cannot unpack the assistant: {e}"))?;
            if count != entry.size() {
                return Err("The assistant archive ended before a file was complete.".into());
            }
            file.flush().and_then(|()| craft_atomic::sync_file(&file)).map_err(|e| format!("Cannot flush the assistant installation: {e}"))?;
        }
    }
    // Tar's zero terminator can precede the gzip checksum; finish bounded decompression to verify it.
    let mut gzip = archive.into_inner();
    std::io::copy(&mut gzip, &mut std::io::sink()).map_err(|e| format!("The assistant archive is damaged: {e}"))?;
    if gzip.limit() == 0 {
        return Err("The assistant archive exceeds its configured expanded size limit.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    const HOST: HostCompatibility = HostCompatibility { protocol: 1, settings: 1, history: 1 };

    fn store(root: &Path) -> Store {
        Store::new(Config {
            root: root.to_path_buf(),
            publisher_team_id: "TESTTEAM".into(),
            signing_identifier: "org.craft.assistant.test".into(),
            limits: Limits::default(),
        })
        .unwrap()
    }

    fn version(value: &str) -> Version {
        Version::parse(value).unwrap()
    }

    fn manifest(value: &str) -> Manifest {
        Manifest {
            version: version(value),
            protocol: Range { min: 1, max: 1 },
            settings: Range { min: 1, max: 1 },
            history: Range { min: 1, max: 1 },
        }
    }

    fn fixture(directory: &Path, manifest: &Manifest) -> (PathBuf, Package) {
        let archive = directory.join(format!("{}.tar.gz", manifest.version));
        let gzip = flate2::write::GzEncoder::new(File::create(&archive).unwrap(), flate2::Compression::default());
        let mut tar = tar::Builder::new(gzip);
        for (path, bytes) in [
            (Path::new(BUNDLE).join(MANIFEST), serde_json::to_vec(manifest).unwrap()),
            (Path::new(BUNDLE).join(EXECUTABLE), b"synthetic helper, never executed".to_vec()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, path, bytes.as_slice()).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        let bytes = std::fs::read(&archive).unwrap();
        (archive, Package { version: manifest.version.clone(), sha256: format!("{:x}", Sha256::digest(bytes)) })
    }

    fn accept(_: &Path, _: &Config, _: bool) -> Result<(), String> {
        Ok(())
    }

    fn refuse(_: &Path, _: &Config, _: bool) -> Result<(), String> {
        Err("synthetic invalid publisher".into())
    }

    #[test]
    fn installs_once_resolves_offline_and_rolls_back_explicitly() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp.path().join("shared"));
        assert!(store.resolve_checked(&HOST, accept).unwrap_err().contains("not installed"));
        let (archive, package) = fixture(temp.path(), &manifest("1.0.0"));
        let first = store.install_checked(&archive, &package, &HOST, accept).unwrap();
        let original = std::fs::read(&first.executable).unwrap();
        assert!(store.install_checked(&archive, &package, &HOST, accept).unwrap_err().contains("immutable"));
        let (archive, package) = fixture(temp.path(), &manifest("1.1.0"));
        store.install_checked(&archive, &package, &HOST, accept).unwrap();
        assert_eq!(store.previous_version().unwrap(), Some(version("1.0.0")));
        std::fs::remove_file(archive).unwrap();
        assert_eq!(store.resolve_checked(&HOST, accept).unwrap().version, version("1.1.0"));
        assert_eq!(std::fs::read(first.executable).unwrap(), original);
        store.activate_checked(&version("1.0.0"), &HOST, accept).unwrap();
        assert_eq!(store.resolve_checked(&HOST, accept).unwrap().version, version("1.0.0"));
        assert_eq!(store.previous_version().unwrap(), Some(version("1.1.0")));
    }

    #[test]
    fn bad_checksum_damaged_archive_and_signature_leave_current_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp.path().join("shared"));
        let (archive, package) = fixture(temp.path(), &manifest("1.0.0"));
        store.install_checked(&archive, &package, &HOST, accept).unwrap();
        let pointer = std::fs::read(store.config.root.join("active.json")).unwrap();
        let (archive, mut package) = fixture(temp.path(), &manifest("1.1.0"));
        let correct = package.sha256.clone();
        package.sha256 = "0".repeat(64);
        assert!(store.install_checked(&archive, &package, &HOST, accept).unwrap_err().contains("checksum"));
        package.sha256 = correct;
        assert!(store.install_checked(&archive, &package, &HOST, refuse).unwrap_err().contains("publisher"));
        std::fs::write(&archive, b"interrupted transfer").unwrap();
        package.sha256 = format!("{:x}", Sha256::digest(b"interrupted transfer"));
        assert!(store.install_checked(&archive, &package, &HOST, accept).is_err());
        assert_eq!(std::fs::read(store.config.root.join("active.json")).unwrap(), pointer);
        assert!(!store.config.root.join("versions/1.1.0").exists());
    }

    #[test]
    fn rejects_incompatible_ranges_and_mismatched_release_before_activation() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp.path().join("shared"));
        for field in ["protocol", "settings", "history"] {
            let mut manifest = manifest("1.0.0");
            let range = match field {
                "protocol" => &mut manifest.protocol,
                "settings" => &mut manifest.settings,
                _ => &mut manifest.history,
            };
            *range = Range { min: 2, max: 1 };
            let (archive, package) = fixture(temp.path(), &manifest);
            let error = store.install_checked(&archive, &package, &HOST, accept).unwrap_err();
            assert!(error.contains(field), "{error}");
            assert!(!store.config.root.join("active.json").exists());
        }
        let (archive, mut package) = fixture(temp.path(), &manifest("1.0.0"));
        package.version = version("9.0.0");
        assert!(store.install_checked(&archive, &package, &HOST, accept).unwrap_err().contains("selected release"));
    }

    #[test]
    fn current_incompatible_release_is_not_silently_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp.path().join("shared"));
        let (archive, package) = fixture(temp.path(), &manifest("1.0.0"));
        store.install_checked(&archive, &package, &HOST, accept).unwrap();
        let other = HostCompatibility { protocol: 2, ..HOST };
        assert!(store.resolve_checked(&other, accept).unwrap_err().contains("update the app"));
        assert_eq!(store.resolve_checked(&HOST, accept).unwrap().version, version("1.0.0"));
    }

    #[test]
    fn concurrent_installs_are_serial_and_never_downgrade() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("shared");
        let barrier = Arc::new(Barrier::new(2));
        let mut workers = Vec::new();
        for value in ["1.0.0", "1.1.0"] {
            let (archive, package) = fixture(temp.path(), &manifest(value));
            let root = root.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                store(&root).install_checked(&archive, &package, &HOST, accept)
            }));
        }
        let results: Vec<_> = workers.into_iter().map(|worker| worker.join().unwrap()).collect();
        assert!(results.iter().any(Result::is_ok));
        assert!(results.iter().filter_map(|result| result.as_ref().err()).all(|error| error.contains("newer assistant")));
        let store = store(&root);
        assert!(root.join("versions/1.1.0").is_dir());
        let current = store.resolve_checked(&HOST, accept).unwrap();
        assert_eq!(current.version, version("1.1.0"));
        assert_ne!(store.previous_version().unwrap(), Some(current.version));
    }

    #[test]
    fn stale_update_cannot_downgrade_a_newer_activation() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp.path().join("shared"));
        let (archive, package) = fixture(temp.path(), &manifest("1.2.0"));
        store.install_checked(&archive, &package, &HOST, accept).unwrap();
        let (archive, package) = fixture(temp.path(), &manifest("1.1.0"));
        assert!(store.install_checked(&archive, &package, &HOST, accept).unwrap_err().contains("Refresh"));
        assert_eq!(store.resolve_checked(&HOST, accept).unwrap().version, version("1.2.0"));
        assert!(!store.config.root.join("versions/1.1.0").exists());
    }

    #[test]
    fn limits_and_orphan_staging_cannot_replace_active_version() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = store(&temp.path().join("shared"));
        let (archive, package) = fixture(temp.path(), &manifest("1.0.0"));
        store.install_checked(&archive, &package, &HOST, accept).unwrap();
        std::fs::create_dir(store.config.root.join(".assistant-stage-interrupted")).unwrap();
        let (archive, package) = fixture(temp.path(), &manifest("1.1.0"));
        store.config.limits.unpacked_bytes = 1;
        assert!(store.install_checked(&archive, &package, &HOST, accept).unwrap_err().contains("size limit"));
        store.config.limits = Limits::default();
        store.config.limits.archive_bytes = 1;
        assert!(store.install_checked(&archive, &package, &HOST, accept).unwrap_err().contains("download limit"));
        assert_eq!(store.resolve_checked(&HOST, accept).unwrap().version, version("1.0.0"));
    }

    #[test]
    fn links_duplicates_and_non_bundle_files_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        for (kind, path, duplicate) in [
            (tar::EntryType::Symlink, "CraftAssistant.app/link", false),
            (tar::EntryType::Link, "CraftAssistant.app/link", false),
            (tar::EntryType::Regular, "other.txt", false),
            (tar::EntryType::Regular, "CraftAssistant.app/file", true),
        ] {
            let archive = temp.path().join("bad.tar.gz");
            let gzip = flate2::write::GzEncoder::new(File::create(&archive).unwrap(), flate2::Compression::default());
            let mut tar = tar::Builder::new(gzip);
            for _ in 0..if duplicate { 2 } else { 1 } {
                let mut header = tar::Header::new_gnu();
                header.set_size(0);
                header.set_entry_type(kind);
                header.set_cksum();
                tar.append_data(&mut header, path, &[][..]).unwrap();
            }
            tar.into_inner().unwrap().finish().unwrap();
            let staging = tempfile::tempdir().unwrap();
            assert!(unpack(File::open(&archive).unwrap(), staging.path(), &Limits::default()).is_err());
        }
    }

    #[test]
    fn gzip_checksum_is_checked_after_tar_end() {
        let temp = tempfile::tempdir().unwrap();
        let (archive, _) = fixture(temp.path(), &manifest("1.0.0"));
        let mut bytes = std::fs::read(&archive).unwrap();
        let crc_offset = bytes.len() - 8;
        bytes[crc_offset] ^= 1;
        std::fs::write(&archive, bytes).unwrap();
        let staging = tempfile::tempdir().unwrap();
        assert!(unpack(File::open(archive).unwrap(), staging.path(), &Limits::default()).is_err());
    }

    #[test]
    fn invalid_config_and_missing_or_corrupt_activation_are_actionable() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = store(temp.path()).config.clone();
        config.publisher_team_id = "injected\" identity".into();
        assert!(Store::new(config).is_err());
        let store = store(&temp.path().join("shared"));
        assert!(store.resolve_checked(&HOST, accept).unwrap_err().contains("Install assistant"));
        std::fs::write(store.config.root.join("active.json"), b"{").unwrap();
        assert!(store.resolve_checked(&HOST, accept).unwrap_err().contains("left untouched"));
    }

    #[test]
    fn corrupt_configuration_cannot_disable_resource_bounds() {
        let temp = tempfile::tempdir().unwrap();
        let original = store(temp.path()).config;
        for field in ["archive_bytes", "unpacked_bytes", "entries", "metadata_bytes", "verification_timeout_secs"] {
            let mut value = serde_json::to_value(&original).unwrap();
            value["limits"][field] = serde_json::json!(if field == "entries" { u64::from(u32::MAX) } else { u64::MAX });
            let config: Config = serde_json::from_value(value).unwrap();
            assert!(Store::new(config).is_err(), "{field} accepted an unbounded limit");
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_installation_paths_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let root = temp.path().join("shared");
        std::os::unix::fs::symlink(other.path(), &root).unwrap();
        let store = store(&root);
        assert!(store.resolve_checked(&HOST, accept).unwrap_err().contains("symbolic link"));
        assert!(!other.path().join("active.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn replaced_bundle_subdirectories_are_refused_before_launch() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp.path().join("shared"));
        let (archive, package) = fixture(temp.path(), &manifest("1.0.0"));
        let installed = store.install_checked(&archive, &package, &HOST, accept).unwrap();
        let contents = installed.bundle.join("Contents");
        let elsewhere = temp.path().join("displaced-contents");
        std::fs::rename(&contents, &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, contents).unwrap();
        assert!(store.resolve_checked(&HOST, accept).unwrap_err().contains("symbolic link"));
    }
}
