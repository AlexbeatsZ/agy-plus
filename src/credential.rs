use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::oauth::parse_payload_metadata;
use crate::paths::{atomic_write_private, AppPaths};

pub trait SystemCredentialStore: Send + Sync {
    fn write(&self, payload_json: &str) -> Result<(), String>;
    fn read(&self) -> Result<Option<String>, String>;
    fn delete(&self) -> Result<(), String>;
    /// Non-prompting existence check (used on macOS before touching Keychain secrets).
    fn has_entry_without_prompt(&self) -> bool;
}

/// Default credential store. When `AGY_PLUS_TEST_KEYRING_FILE` is set (for isolated
/// integration tests only), uses a local file instead of the OS Keychain / Credential Manager.
pub fn default_system_store() -> Box<dyn SystemCredentialStore> {
    if let Some(path) = std::env::var_os("AGY_PLUS_TEST_KEYRING_FILE").filter(|v| !v.is_empty()) {
        return Box::new(FileBackedTestStore {
            path: PathBuf::from(path),
        });
    }
    Box::new(OsCredentialStore)
}

struct FileBackedTestStore {
    path: PathBuf,
}

impl SystemCredentialStore for FileBackedTestStore {
    fn write(&self, payload_json: &str) -> Result<(), String> {
        atomic_write_private(&self.path, payload_json.as_bytes())
    }

    fn read(&self) -> Result<Option<String>, String> {
        match fs::read_to_string(&self.path) {
            Ok(content) if !content.trim().is_empty() => Ok(Some(content)),
            Ok(_) => Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("Failed to read test keyring file: {error}")),
        }
    }

    fn delete(&self) -> Result<(), String> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("Failed to remove test keyring file: {error}")),
        }
    }

    fn has_entry_without_prompt(&self) -> bool {
        self.path.is_file()
    }
}

pub struct OsCredentialStore;

impl SystemCredentialStore for OsCredentialStore {
    fn write(&self, payload_json: &str) -> Result<(), String> {
        write_antigravity_system_payload(payload_json)
    }

    fn read(&self) -> Result<Option<String>, String> {
        read_antigravity_system_payload()
    }

    fn delete(&self) -> Result<(), String> {
        delete_antigravity_system_payload()
    }

    fn has_entry_without_prompt(&self) -> bool {
        has_antigravity_system_entry()
    }
}

#[derive(Debug, Clone)]
pub struct LiveCredentialBackup {
    pub system_payload: Option<String>,
    pub cli_file: Option<Vec<u8>>,
    pub desktop_file: Option<Vec<u8>>,
}

/// Writes system credential first, then CLI token file, then desktop fallback token file.
/// Reports partial-success errors accurately if a later write fails.
pub fn persist_live_credential(
    paths: &AppPaths,
    payload_json: &str,
    store: &dyn SystemCredentialStore,
) -> Result<(), String> {
    store.write(payload_json)?;
    atomic_write_private(&paths.cli_token_path(), payload_json.as_bytes()).map_err(|e| {
        format!("System credential was updated, but agy CLI token file failed (please retry): {e}")
    })?;
    atomic_write_private(&paths.desktop_token_path(), payload_json.as_bytes()).map_err(|e| {
        format!(
            "System and CLI credentials were updated, but desktop fallback file failed (please retry): {e}"
        )
    })?;
    Ok(())
}

pub fn clear_live_credential(
    paths: &AppPaths,
    store: &dyn SystemCredentialStore,
) -> Result<(), String> {
    store.delete()?;
    remove_if_exists(&paths.cli_token_path()).map_err(|e| {
        format!("System credential cleared, but failed to remove CLI token file: {e}")
    })?;
    remove_if_exists(&paths.desktop_token_path()).map_err(|e| {
        format!("System and CLI credentials cleared, but failed to remove desktop token file: {e}")
    })?;
    Ok(())
}

pub fn capture_live_backup(
    paths: &AppPaths,
    store: &dyn SystemCredentialStore,
    allow_system_read: bool,
) -> LiveCredentialBackup {
    let cli_file = fs::read(paths.cli_token_path()).ok();
    let desktop_file = fs::read(paths.desktop_token_path()).ok();
    let system_payload = if cfg!(windows) || allow_system_read {
        store.read().ok().flatten()
    } else {
        // Avoid triggering an unnecessary macOS Keychain read prompt when files already hold the credential.
        desktop_file
            .as_deref()
            .or(cli_file.as_deref())
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .map(str::to_owned)
    };
    LiveCredentialBackup {
        system_payload,
        cli_file,
        desktop_file,
    }
}

pub fn restore_live_backup(
    paths: &AppPaths,
    backup: &LiveCredentialBackup,
    store: &dyn SystemCredentialStore,
) -> Result<(), String> {
    let mut errors = Vec::new();
    match &backup.system_payload {
        Some(payload) => {
            if let Err(err) = store.write(payload) {
                errors.push(err);
            }
        }
        None => {
            if let Err(err) = store.delete() {
                errors.push(err);
            }
        }
    }
    if let Err(err) = restore_optional_file(&paths.cli_token_path(), backup.cli_file.as_deref()) {
        errors.push(err);
    }
    if let Err(err) =
        restore_optional_file(&paths.desktop_token_path(), backup.desktop_file.as_deref())
    {
        errors.push(err);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Reads the currently active Antigravity OAuth JSON payload.
///
/// - On Windows, reads Credential Manager directly (prompt-free) and falls back to token files.
/// - On macOS/Linux, prefers the newer valid token file (`jetski-standalone-oauth-token` /
///   `antigravity-oauth-token`) so routine calls never trigger a macOS Keychain read prompt.
///   Falls back to the system keyring only when files are missing or disagree with `expected_email`
///   and a keyring entry is confirmed to exist.
pub fn read_live_payload(
    paths: &AppPaths,
    store: &dyn SystemCredentialStore,
    expected_email: Option<&str>,
    allow_system_fallback: bool,
) -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        if let Ok(Some(payload)) = store.read() {
            if parse_payload_metadata(&payload).is_ok() {
                return Ok(Some(payload));
            }
        }
    }

    let file_candidate = read_newest_valid_token_file(paths);
    let expected_norm = expected_email
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());

    if let Some(ref payload) = file_candidate {
        let email_matches = match (&expected_norm, parse_payload_metadata(payload).ok()) {
            (Some(expected), Some(parsed)) => {
                parsed
                    .email
                    .as_deref()
                    .map(|value| value.to_ascii_lowercase())
                    .as_deref()
                    == Some(expected.as_str())
            }
            _ => true,
        };
        if email_matches {
            return Ok(Some(payload.clone()));
        }
    }

    let should_query_system =
        allow_system_fallback && (file_candidate.is_none() || expected_norm.is_some());
    if should_query_system && store.has_entry_without_prompt() {
        if let Ok(Some(payload)) = store.read() {
            if parse_payload_metadata(&payload).is_ok() {
                return Ok(Some(payload));
            }
        }
    }

    Ok(file_candidate)
}

fn read_newest_valid_token_file(paths: &AppPaths) -> Option<String> {
    let candidates = [paths.desktop_token_path(), paths.cli_token_path()];
    let mut best: Option<(SystemTime, String)> = None;
    for path in candidates {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        if parse_payload_metadata(&content).is_err() {
            continue;
        }
        let mtime = fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if best
            .as_ref()
            .is_none_or(|(best_time, _)| mtime >= *best_time)
        {
            best = Some((mtime, content));
        }
    }
    best.map(|(_, content)| content)
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Failed to delete {}: {error}", path.display())),
    }
}

fn restore_optional_file(path: &Path, bytes: Option<&[u8]>) -> Result<(), String> {
    match bytes {
        Some(content) => atomic_write_private(path, content),
        None => remove_if_exists(path),
    }
}

#[cfg(target_os = "macos")]
fn decode_go_keyring_value(raw: &str) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let trimmed = raw.trim();
    if let Some(encoded) = trimmed.strip_prefix("go-keyring-base64:") {
        let bytes = STANDARD
            .decode(encoded.trim())
            .map_err(|_| "Failed to decode macOS Keychain go-keyring-base64 payload".to_string())?;
        String::from_utf8(bytes)
            .map_err(|_| "macOS Keychain payload is not valid UTF-8".to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

#[cfg(target_os = "windows")]
fn validate_windows_credential_size(size: usize) -> Result<(), String> {
    const CRED_MAX_CREDENTIAL_BLOB_SIZE: usize = 5 * 512;
    if size > CRED_MAX_CREDENTIAL_BLOB_SIZE {
        return Err("ANTIGRAVITY_WINDOWS_CREDENTIAL_TOO_LARGE".to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_credential_write_error(error: std::io::Error) -> String {
    const ERROR_NOT_ENOUGH_MEMORY: i32 = 8;
    if error.raw_os_error() == Some(ERROR_NOT_ENOUGH_MEMORY) {
        return "ANTIGRAVITY_WINDOWS_CREDENTIAL_STORE_CAPACITY".to_string();
    }
    format!("Failed to write Windows Credential Manager: {error}")
}

#[cfg(target_os = "windows")]
mod win_cred {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt as _;
    use std::ptr;

    #[repr(C)]
    pub struct FileTime {
        pub dw_low_date_time: u32,
        pub dw_high_date_time: u32,
    }

    #[repr(C)]
    pub struct CredentialW {
        pub flags: u32,
        pub cred_type: u32,
        pub target_name: *const u16,
        pub comment: *const u16,
        pub last_written: FileTime,
        pub credential_blob_size: u32,
        pub credential_blob: *const u8,
        pub persist: u32,
        pub attribute_count: u32,
        pub attributes: *const std::ffi::c_void,
        pub target_alias: *const u16,
        pub user_name: *const u16,
    }

    pub const CRED_TYPE_GENERIC: u32 = 1;
    pub const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;
    pub const ERROR_NOT_FOUND: i32 = 1168;

    #[link(name = "advapi32")]
    extern "system" {
        pub fn CredWriteW(credential: *const CredentialW, flags: u32) -> i32;
        pub fn CredReadW(
            target_name: *const u16,
            cred_type: u32,
            flags: u32,
            credential: *mut *mut CredentialW,
        ) -> i32;
        pub fn CredDeleteW(target_name: *const u16, cred_type: u32, flags: u32) -> i32;
        pub fn CredFree(buffer: *mut std::ffi::c_void);
    }

    pub fn wide(value: &str) -> Vec<u16> {
        OsStr::new(value)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    pub fn write(payload_json: &str) -> Result<(), String> {
        let target_wide = wide("gemini:antigravity");
        let user_wide = wide("antigravity");
        let secret = payload_json.as_bytes();
        super::validate_windows_credential_size(secret.len())?;

        let credential = CredentialW {
            flags: 0,
            cred_type: CRED_TYPE_GENERIC,
            target_name: target_wide.as_ptr(),
            comment: ptr::null(),
            last_written: FileTime {
                dw_low_date_time: 0,
                dw_high_date_time: 0,
            },
            credential_blob_size: secret.len() as u32,
            credential_blob: secret.as_ptr(),
            persist: CRED_PERSIST_LOCAL_MACHINE,
            attribute_count: 0,
            attributes: ptr::null(),
            target_alias: ptr::null(),
            user_name: user_wide.as_ptr(),
        };

        unsafe {
            if CredWriteW(&credential, 0) == 0 {
                return Err(super::windows_credential_write_error(
                    std::io::Error::last_os_error(),
                ));
            }
        }
        Ok(())
    }

    pub fn read() -> Result<Option<String>, String> {
        let target_wide = wide("gemini:antigravity");
        let mut cred_ptr: *mut CredentialW = ptr::null_mut();
        unsafe {
            if CredReadW(target_wide.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred_ptr) == 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(ERROR_NOT_FOUND) {
                    return Ok(None);
                }
                return Err(format!("Failed to read Windows Credential Manager: {err}"));
            }
            if cred_ptr.is_null() {
                return Ok(None);
            }
            let cred = &*cred_ptr;
            let result = if cred.credential_blob.is_null() || cred.credential_blob_size == 0 {
                Ok(None)
            } else {
                let slice = std::slice::from_raw_parts(
                    cred.credential_blob,
                    cred.credential_blob_size as usize,
                );
                String::from_utf8(slice.to_vec()).map(Some).map_err(|_| {
                    "Windows Credential Manager payload is not valid UTF-8".to_string()
                })
            };
            CredFree(cred_ptr.cast());
            result
        }
    }

    pub fn delete() -> Result<(), String> {
        let target_wide = wide("gemini:antigravity");
        unsafe {
            if CredDeleteW(target_wide.as_ptr(), CRED_TYPE_GENERIC, 0) == 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(ERROR_NOT_FOUND) {
                    return Ok(());
                }
                return Err(format!(
                    "Failed to delete Windows Credential Manager entry: {err}"
                ));
            }
        }
        Ok(())
    }
}

pub fn write_antigravity_system_payload(payload_json: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    write_antigravity_keychain_payload(
        payload_json,
        std::ffi::OsStr::new("security"),
        std::time::Duration::from_secs(15),
    )?;

    #[cfg(target_os = "windows")]
    win_cred::write(payload_json)?;

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;

        let mut command = Command::new("secret-tool");
        command.args([
            "store",
            "--label=gemini",
            "service",
            "gemini",
            "username",
            "antigravity",
        ]);
        let output = crate::process_timeout::output_with_input_timeout(
            &mut command,
            payload_json.as_bytes(),
            std::time::Duration::from_secs(15),
        )
        .map_err(|e| format!("Failed to execute Linux secret-tool store: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Linux secret-tool store failed: {}", stderr.trim()));
        }
    }

    Ok(())
}

pub fn read_antigravity_system_payload() -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        let mut command = Command::new("security");
        command.args([
            "find-generic-password",
            "-s",
            "gemini",
            "-a",
            "antigravity",
            "-w",
        ]);
        let output = crate::process_timeout::output_with_timeout(
            &mut command,
            std::time::Duration::from_secs(15),
        )
        .map_err(|e| format!("Failed to execute macOS Keychain lookup: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if output.status.code() == Some(44) || stderr.contains("could not be found") {
                return Ok(None);
            }
            return Err(format!("macOS Keychain lookup failed: {}", stderr.trim()));
        }
        let raw = String::from_utf8_lossy(&output.stdout);
        if raw.trim().is_empty() {
            return Ok(None);
        }
        decode_go_keyring_value(&raw).map(Some)
    }

    #[cfg(target_os = "windows")]
    {
        win_cred::read()
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;

        let mut command = Command::new("secret-tool");
        command.args(["lookup", "service", "gemini", "username", "antigravity"]);
        let output = crate::process_timeout::output_with_timeout(
            &mut command,
            std::time::Duration::from_secs(15),
        )
        .map_err(|e| format!("Failed to execute Linux secret-tool lookup: {e}"))?;
        if !output.status.success() {
            return Ok(None);
        }
        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if raw.is_empty() {
            Ok(None)
        } else {
            Ok(Some(raw))
        }
    }
}

pub fn delete_antigravity_system_payload() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        let mut command = Command::new("security");
        command.args([
            "delete-generic-password",
            "-s",
            "gemini",
            "-a",
            "antigravity",
        ]);
        let output = crate::process_timeout::output_with_timeout(
            &mut command,
            std::time::Duration::from_secs(15),
        )
        .map_err(|e| format!("Failed to execute macOS Keychain delete: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if output.status.code() == Some(44) || stderr.contains("could not be found") {
                return Ok(());
            }
            return Err(format!("macOS Keychain delete failed: {}", stderr.trim()));
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        win_cred::delete()
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;

        let mut command = Command::new("secret-tool");
        command.args(["clear", "service", "gemini", "username", "antigravity"]);
        let output = crate::process_timeout::output_with_timeout(
            &mut command,
            std::time::Duration::from_secs(15),
        )
        .map_err(|e| format!("Failed to execute Linux secret-tool clear: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Linux secret-tool clear failed: {}", stderr.trim()));
        }
        Ok(())
    }
}

pub fn has_antigravity_system_entry() -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        // Metadata-only lookup without `-w` never triggers a Keychain secret authorization prompt.
        let mut command = Command::new("security");
        command.args(["find-generic-password", "-s", "gemini", "-a", "antigravity"]);
        crate::process_timeout::output_with_timeout(&mut command, std::time::Duration::from_secs(5))
            .is_ok_and(|output| output.status.success())
    }

    #[cfg(target_os = "windows")]
    {
        win_cred::read().ok().flatten().is_some()
    }

    #[cfg(target_os = "linux")]
    {
        read_antigravity_system_payload().ok().flatten().is_some()
    }
}

#[cfg(target_os = "macos")]
fn write_antigravity_keychain_payload(
    payload_json: &str,
    program: &std::ffi::OsStr,
    timeout: std::time::Duration,
) -> Result<(), String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use std::process::Command;

    let keychain_value = format!("go-keyring-base64:{}", STANDARD.encode(payload_json));
    // Update the credential in place and preserve its existing access policy.
    // Passing -A or -T also calls SecKeychainItemSetAccess, prompting on every switch.
    let mut command = Command::new(program);
    command.args([
        "add-generic-password",
        "-U",
        "-s",
        "gemini",
        "-a",
        "antigravity",
        "-w",
        &keychain_value,
    ]);
    let output = crate::process_timeout::output_with_timeout(&mut command, timeout)
        .map_err(|e| format!("Failed to execute macOS Keychain write: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to write macOS Keychain: {}", stderr.trim()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct MemoryStore {
        value: Mutex<Option<String>>,
        fail_write: bool,
        reads: Mutex<usize>,
    }

    impl MemoryStore {
        fn new(initial: Option<&str>, fail_write: bool) -> Self {
            Self {
                value: Mutex::new(initial.map(str::to_owned)),
                fail_write,
                reads: Mutex::new(0),
            }
        }
        fn read_count(&self) -> usize {
            *self.reads.lock().unwrap()
        }
    }

    impl SystemCredentialStore for MemoryStore {
        fn write(&self, payload_json: &str) -> Result<(), String> {
            if self.fail_write {
                return Err("denied".into());
            }
            *self.value.lock().unwrap() = Some(payload_json.to_string());
            Ok(())
        }
        fn read(&self) -> Result<Option<String>, String> {
            *self.reads.lock().unwrap() += 1;
            Ok(self.value.lock().unwrap().clone())
        }
        fn delete(&self) -> Result<(), String> {
            *self.value.lock().unwrap() = None;
            Ok(())
        }
        fn has_entry_without_prompt(&self) -> bool {
            self.value.lock().unwrap().is_some()
        }
    }

    fn sample_payload(email: &str, sub: &str) -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256"}"#);
        let claims = URL_SAFE_NO_PAD.encode(
            serde_json::json!({"sub": sub, "email": email, "name": "Test User"}).to_string(),
        );
        let id_token = format!("{header}.{claims}.sig");
        serde_json::json!({
            "auth_method": "consumer",
            "id_token": id_token,
            "token": {
                "access_token": "ya29.test",
                "token_type": "Bearer",
                "refresh_token": "1//test-rt",
                "expiry": "2026-11-26T15:00:00.000000Z"
            }
        })
        .to_string()
    }

    #[test]
    fn creates_cli_session_on_first_login_and_replaces_old_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            home_dir: dir.path().join("home"),
            state_dir: dir.path().join("state"),
        };
        let store = MemoryStore::new(None, false);
        let old = sample_payload("old@example.com", "1");
        let new = sample_payload("new@example.com", "2");
        persist_live_credential(&paths, &old, &store).unwrap();
        persist_live_credential(&paths, &new, &store).unwrap();
        assert_eq!(fs::read_to_string(paths.cli_token_path()).unwrap(), new);
        assert_eq!(fs::read_to_string(paths.desktop_token_path()).unwrap(), new);
        assert_eq!(store.read().unwrap().as_deref(), Some(new.as_str()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(paths.cli_token_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn system_failure_leaves_files_untouched_and_partial_write_reports_error() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            home_dir: dir.path().join("home"),
            state_dir: dir.path().join("state"),
        };
        let failing = MemoryStore::new(None, true);
        assert!(persist_live_credential(&paths, "secret", &failing).is_err());
        assert!(!paths.gemini_dir().exists());

        fs::create_dir_all(paths.gemini_dir()).unwrap();
        fs::write(paths.gemini_dir().join("antigravity-cli"), "blocking").unwrap();
        let ok_store = MemoryStore::new(None, false);
        assert!(persist_live_credential(&paths, "secret", &ok_store)
            .unwrap_err()
            .contains("CLI"));
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_live_read_avoids_keychain_when_file_matches_expected_email() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            home_dir: dir.path().join("home"),
            state_dir: dir.path().join("state"),
        };
        let file_payload = sample_payload("alice@example.com", "101");
        let keychain_payload = sample_payload("bob@example.com", "202");
        atomic_write_private(&paths.desktop_token_path(), file_payload.as_bytes()).unwrap();
        let store = MemoryStore::new(Some(&keychain_payload), false);

        // Matching expected email must not read Keychain.
        let read = read_live_payload(&paths, &store, Some("alice@example.com"), true).unwrap();
        assert_eq!(read.as_deref(), Some(file_payload.as_str()));
        assert_eq!(store.read_count(), 0);

        // Divergent expected email falls back to Keychain.
        let read = read_live_payload(&paths, &store, Some("bob@example.com"), true).unwrap();
        assert_eq!(read.as_deref(), Some(keychain_payload.as_str()));
        assert_eq!(store.read_count(), 1);
    }
}
