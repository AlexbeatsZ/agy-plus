use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub home_dir: PathBuf,
    pub state_dir: PathBuf,
}

impl AppPaths {
    pub fn discover() -> anyhow::Result<Self> {
        let home_dir = std::env::var_os("AGY_PLUS_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(dirs::home_dir)
            .ok_or_else(|| anyhow::anyhow!("Unable to locate user home directory"))?;

        let state_dir = std::env::var_os("AGY_PLUS_STATE_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| dirs::data_local_dir().map(|dir| dir.join("agy-plus")))
            .ok_or_else(|| {
                anyhow::anyhow!("Unable to locate AGY Plus application data directory")
            })?;

        Ok(Self {
            home_dir,
            state_dir,
        })
    }

    pub fn gemini_dir(&self) -> PathBuf {
        self.home_dir.join(".gemini")
    }

    pub fn desktop_token_path(&self) -> PathBuf {
        self.gemini_dir().join("jetski-standalone-oauth-token")
    }

    pub fn cli_token_path(&self) -> PathBuf {
        self.gemini_dir()
            .join("antigravity-cli")
            .join("antigravity-oauth-token")
    }

    pub fn accounts_dir(&self) -> PathBuf {
        self.state_dir.join("accounts")
    }

    pub fn vault_lock_path(&self) -> PathBuf {
        self.accounts_dir().join("vault.lock")
    }

    pub fn launcher_lock_path(&self) -> PathBuf {
        self.state_dir.join("launcher.lock")
    }

    pub fn pending_action_path(&self) -> PathBuf {
        self.state_dir.join("pending-action.json")
    }

    pub fn log_path(&self) -> PathBuf {
        self.state_dir.join("agy-plus.log")
    }
}

/// Atomically writes sensitive bytes using a same-directory tempfile + sync + persist.
/// On Unix, `tempfile::Builder` creates files with mode 0600.
pub fn atomic_write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Invalid target path: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create directory {}: {e}", parent.display()))?;
    #[cfg(unix)]
    restrict_dir_permissions(parent);

    let mut file = tempfile::Builder::new()
        .prefix(".agy-plus-")
        .tempfile_in(parent)
        .map_err(|e| {
            format!(
                "Failed to create temporary file in {}: {e}",
                parent.display()
            )
        })?;
    file.write_all(bytes)
        .and_then(|_| file.as_file().sync_all())
        .map_err(|e| format!("Failed to write temporary file for {}: {e}", path.display()))?;
    file.persist(path)
        .map_err(|e| format!("Failed to replace {}: {}", path.display(), e.error))?;
    Ok(())
}

#[cfg(unix)]
fn restrict_dir_permissions(dir: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    // Only tighten AGY Plus's own state/vault directories; avoid altering ~/.gemini unexpectedly.
    if dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == "accounts" || n == "agy-plus")
    {
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
}
