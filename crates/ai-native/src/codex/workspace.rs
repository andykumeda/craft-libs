//! Native workspace choice is explicit and independent of the GUI process working directory.
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use craft_ai_core::{AiError, AppId};

/// Stable app-specific output directory. Normal Codex account/profile discovery is separate.
pub fn workspace_for_app(root: &Path, app: &AppId) -> Result<PathBuf, AiError> {
    let history = crate::storage::HistoryStore::new(root, app)?;
    let name = history.root().file_name().ok_or_else(|| AiError::invalid("Invalid Craft workspace app identifier."))?;
    Ok(root.join("workspaces").join(name))
}

pub(super) fn selected(default: Option<&Path>) -> Result<PathBuf, AiError> {
    let default = match default {
        Some(path) => path.to_owned(),
        None => crate::storage::SharedStore::default_root()?.join("workspaces").join("codex"),
    };
    resolve(std::env::var_os("CRAFT_CODEX_WORKSPACE").as_deref(), default)
}

fn resolve(override_path: Option<&OsStr>, default: PathBuf) -> Result<PathBuf, AiError> {
    let path = override_path.map_or(default, PathBuf::from);
    validate(&path)?;
    Ok(path)
}

fn validate(path: &Path) -> Result<(), AiError> {
    if !path.is_absolute() || path.parent().is_none() || path.components().any(|component| matches!(component, Component::ParentDir)) {
        return Err(AiError::invalid(
            "Choose an absolute Codex workspace directory other than the filesystem root, without '..'; set CRAFT_CODEX_WORKSPACE explicitly.",
        ));
    }
    if path.components().any(|component| match component {
        Component::Normal(name) => Path::new(name).extension().is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("app")),
        _ => false,
    }) {
        return Err(AiError::invalid(
            "The Codex workspace cannot be inside an application bundle. Choose a project/output directory with CRAFT_CODEX_WORKSPACE.",
        ));
    }
    Ok(())
}

pub(super) fn prepare(path: PathBuf) -> Result<PathBuf, AiError> {
    validate(&path)?;
    // Resolve existing ancestors before creating anything, including symlinks into app bundles.
    let path = path
        .ancestors()
        .find_map(|ancestor| match ancestor.canonicalize() {
            Ok(canonical) => Some(
                path.strip_prefix(ancestor)
                    .map(|suffix| canonical.join(suffix))
                    .map_err(|_| AiError::invalid("Could not resolve the Codex workspace path.")),
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => Some(Err(AiError::invalid("Could not inspect the selected Codex workspace directory."))),
        })
        .ok_or_else(|| AiError::invalid("No workspace ancestor could be resolved."))??;
    validate(&path)?;
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    // Recursive creation preserves permissions of existing user projects.
    builder
        .create(&path)
        .map_err(|_| AiError::invalid("Could not create the selected Codex workspace directory. Check CRAFT_CODEX_WORKSPACE and its permissions."))?;
    let canonical = path.canonicalize().map_err(|_| AiError::invalid("Could not resolve the selected Codex workspace directory."))?;
    validate(&canonical)?;
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("craft-native-workspace-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn override_precedence_invalid_paths_and_app_separation_are_explicit() {
        let temp = Temp::new();
        let default = workspace_for_app(&temp.0, &AppId::new("deckcraft").unwrap()).unwrap();
        assert_eq!(resolve(None, default.clone()).unwrap(), default);
        let explicit = temp.0.join("chosen-project");
        assert_eq!(resolve(Some(explicit.as_os_str()), default.clone()).unwrap(), explicit);
        assert_ne!(default, workspace_for_app(&temp.0, &AppId::new("wordcraft").unwrap()).unwrap());
        for invalid in ["", "relative", "/", "/tmp/..", "/Applications/Example.app/Contents"] {
            assert!(resolve(Some(OsStr::new(invalid)), default.clone()).is_err(), "{invalid}");
        }
        assert!(workspace_for_app(&temp.0, &AppId::new("../unsafe").unwrap()).is_err());
    }
    #[test]
    fn finder_root_cwd_never_becomes_the_default_workspace() {
        const CHILD: &str = "CRAFT_WORKSPACE_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let temp = Temp::new();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "codex::workspace::tests::finder_root_cwd_never_becomes_the_default_workspace", "--nocapture"])
                .current_dir(if cfg!(windows) { std::env::temp_dir() } else { PathBuf::from("/") })
                .env(CHILD, "1")
                .env("CRAFT_AI_CONFIG_DIR", &temp.0)
                .env_remove("CRAFT_CODEX_WORKSPACE")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        let selected = selected(None).unwrap();
        assert_ne!(selected, std::env::current_dir().unwrap());
        let expected = std::env::var_os("CRAFT_AI_CONFIG_DIR").map(PathBuf::from).unwrap().join("workspaces/codex");
        assert_eq!(selected, expected);
        let working = super::super::WorkingDirectory::selected(None).unwrap();
        let prepared = working.0.clone();
        assert_eq!(prepared, selected.canonicalize().unwrap());
        std::fs::write(prepared.join("retained-output.txt"), "synthetic output").unwrap();
        drop(working);
        assert!(prepared.join("retained-output.txt").is_file());
    }
    #[test]
    #[cfg(unix)]
    fn existing_permissions_are_preserved_and_root_symlinks_are_refused() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let temp = Temp::new();
        let existing = temp.0.join("existing");
        std::fs::create_dir(&existing).unwrap();
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o750)).unwrap();
        prepare(existing.clone()).unwrap();
        assert_eq!(std::fs::metadata(existing).unwrap().permissions().mode() & 0o777, 0o750);
        let link = temp.0.join("root-link");
        symlink("/", &link).unwrap();
        assert!(prepare(link).is_err());
        let bundle = temp.0.join("Example.app");
        std::fs::create_dir(&bundle).unwrap();
        let link = temp.0.join("bundle-link");
        symlink(&bundle, &link).unwrap();
        assert!(prepare(link.join("must-not-be-created")).is_err());
        assert!(!bundle.join("must-not-be-created").exists());
        let created = prepare(temp.0.join("created/private")).unwrap();
        assert_eq!(std::fs::metadata(created).unwrap().permissions().mode() & 0o777, 0o700);
    }
}
