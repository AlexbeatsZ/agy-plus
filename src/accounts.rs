use std::collections::HashSet;
use std::fs::{self, File};
use std::io;
use std::path::Path;

use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::credential::{
    clear_live_credential, persist_live_credential, read_live_payload, SystemCredentialStore,
};
use crate::oauth::{parse_payload_metadata, ExchangedCredential};
use crate::paths::{atomic_write_private, AppPaths};

const ACCOUNT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum AccountError {
    #[error("Invalid account identifier")]
    InvalidRecordId,
    #[error("Saved account was not found")]
    NotFound,
    #[error("Cannot remove the currently active account. Switch to another account first.")]
    ActiveAccount,
    #[error("Account storage failed: {0}")]
    Storage(String),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, AccountError>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AccountSource {
    OAuth,
    Rt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredAccount {
    pub schema_version: u32,
    pub record_id: String,
    pub source: AccountSource,
    pub email: Option<String>,
    pub name: Option<String>,
    pub subject: Option<String>,
    pub saved_at: String,
    pub last_used_at: String,
    pub payload_json: String,
}

/// Public summary sent to the injected renderer. Never contains tokens or raw payloads.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummary {
    pub record_id: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub source: AccountSource,
    pub active: bool,
    pub saved_at: String,
    pub last_used_at: String,
}

impl StoredAccount {
    pub fn summary(&self, active: bool) -> AccountSummary {
        AccountSummary {
            record_id: self.record_id.clone(),
            email: self.email.clone(),
            name: self.name.clone(),
            source: self.source,
            active,
            saved_at: self.saved_at.clone(),
            last_used_at: self.last_used_at.clone(),
        }
    }
}

pub struct AccountVault {
    paths: AppPaths,
}

impl AccountVault {
    pub fn new(paths: AppPaths) -> Self {
        Self { paths }
    }

    pub fn list_accounts(
        &self,
        store: &dyn SystemCredentialStore,
        expected_email: Option<&str>,
    ) -> Result<Vec<AccountSummary>> {
        let _lock = self.lock_vault()?;
        let live_id = self.snapshot_live_unlocked(store, expected_email, true)?;
        self.list_unlocked(live_id.as_deref(), expected_email)
    }

    pub fn save_rt_account(
        &self,
        store: &dyn SystemCredentialStore,
        exchanged: ExchangedCredential,
    ) -> Result<AccountSummary> {
        validate_record_id(&exchanged.record_id)?;
        let _lock = self.lock_vault()?;
        let live_id = self.snapshot_live_unlocked(store, None, false)?;
        let now = now_iso();
        let existing = self.load_optional_unlocked(&exchanged.record_id)?;
        let stored = StoredAccount {
            schema_version: ACCOUNT_SCHEMA_VERSION,
            record_id: exchanged.record_id.clone(),
            source: AccountSource::Rt,
            email: exchanged
                .email
                .or_else(|| existing.as_ref().and_then(|item| item.email.clone())),
            name: exchanged
                .name
                .or_else(|| existing.as_ref().and_then(|item| item.name.clone())),
            subject: exchanged
                .subject
                .or_else(|| existing.as_ref().and_then(|item| item.subject.clone())),
            saved_at: existing
                .as_ref()
                .map(|item| item.saved_at.clone())
                .unwrap_or_else(|| now.clone()),
            last_used_at: now,
            payload_json: exchanged.payload_json,
        };
        self.store_unlocked(&stored)?;
        let active = live_id.as_deref() == Some(stored.record_id.as_str());
        Ok(stored.summary(active))
    }

    pub fn delete_account(
        &self,
        store: &dyn SystemCredentialStore,
        record_id: &str,
        expected_email: Option<&str>,
    ) -> Result<()> {
        validate_record_id(record_id)?;
        let _lock = self.lock_vault()?;
        let target = self.load_unlocked(record_id)?;
        let live_id = self.snapshot_live_unlocked(store, expected_email, false)?;
        let active_by_id = live_id.as_deref() == Some(record_id);
        let active_by_email = expected_email
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .is_some_and(|expected| {
                target
                    .email
                    .as_deref()
                    .is_some_and(|email| email.eq_ignore_ascii_case(expected))
            });
        if active_by_id || active_by_email {
            return Err(AccountError::ActiveAccount);
        }
        let path = self.account_path(record_id);
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(AccountError::NotFound),
            Err(error) => Err(error.into()),
        }
    }

    pub fn validate_switch(&self, record_id: &str) -> Result<StoredAccount> {
        validate_record_id(record_id)?;
        let _lock = self.lock_vault()?;
        let account = self.load_unlocked(record_id)?;
        parse_payload_metadata(&account.payload_json).map_err(AccountError::Storage)?;
        Ok(account)
    }

    pub fn switch_after_shutdown(
        &self,
        store: &dyn SystemCredentialStore,
        record_id: &str,
    ) -> Result<AccountSummary> {
        validate_record_id(record_id)?;
        let _lock = self.lock_vault()?;
        let mut target = self.load_unlocked(record_id)?;
        parse_payload_metadata(&target.payload_json).map_err(AccountError::Storage)?;
        // Always preserve the currently signed-in account in the vault before replacing it.
        let _ = self.snapshot_live_unlocked(store, None, false);
        persist_live_credential(&self.paths, &target.payload_json, store)
            .map_err(AccountError::Storage)?;
        target.last_used_at = now_iso();
        self.store_unlocked(&target)?;
        Ok(target.summary(true))
    }

    pub fn begin_google_login_after_shutdown(
        &self,
        store: &dyn SystemCredentialStore,
    ) -> Result<()> {
        let _lock = self.lock_vault()?;
        let _ = self.snapshot_live_unlocked(store, None, false);
        clear_live_credential(&self.paths, store).map_err(AccountError::Storage)?;
        Ok(())
    }

    pub fn capture_live_account(
        &self,
        store: &dyn SystemCredentialStore,
        allow_system_fallback: bool,
    ) -> Result<Option<AccountSummary>> {
        let _lock = self.lock_vault()?;
        let Some(record_id) = self.snapshot_live_unlocked(store, None, allow_system_fallback)?
        else {
            return Ok(None);
        };
        let account = self.load_unlocked(&record_id)?;
        Ok(Some(account.summary(true)))
    }

    fn snapshot_live_unlocked(
        &self,
        store: &dyn SystemCredentialStore,
        expected_email: Option<&str>,
        allow_system_fallback: bool,
    ) -> Result<Option<String>> {
        let Some(payload_json) =
            read_live_payload(&self.paths, store, expected_email, allow_system_fallback)
                .map_err(AccountError::Storage)?
        else {
            return Ok(None);
        };
        let parsed = match parse_payload_metadata(&payload_json) {
            Ok(parsed) => parsed,
            Err(_) => return Ok(None),
        };
        let now = now_iso();
        let existing = self.load_optional_unlocked(&parsed.record_id)?;
        let stored = StoredAccount {
            schema_version: ACCOUNT_SCHEMA_VERSION,
            record_id: parsed.record_id.clone(),
            // Preserve RT provenance if this account was originally added via RT.
            source: existing
                .as_ref()
                .map(|item| item.source)
                .unwrap_or(AccountSource::OAuth),
            email: parsed
                .email
                .or_else(|| existing.as_ref().and_then(|item| item.email.clone()))
                .or_else(|| {
                    expected_email
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                }),
            name: parsed
                .name
                .or_else(|| existing.as_ref().and_then(|item| item.name.clone())),
            subject: parsed
                .subject
                .or_else(|| existing.as_ref().and_then(|item| item.subject.clone())),
            saved_at: existing
                .as_ref()
                .map(|item| item.saved_at.clone())
                .unwrap_or_else(|| now.clone()),
            last_used_at: existing
                .as_ref()
                .map(|item| item.last_used_at.clone())
                .unwrap_or(now),
            payload_json,
        };
        self.store_unlocked(&stored)?;
        Ok(Some(stored.record_id))
    }

    fn list_unlocked(
        &self,
        live_record_id: Option<&str>,
        expected_email: Option<&str>,
    ) -> Result<Vec<AccountSummary>> {
        let dir = self.paths.accounts_dir();
        ensure_dir(&dir)?;
        let expected_norm = expected_email
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_ascii_lowercase());
        let mut accounts = Vec::new();
        let mut seen = HashSet::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file()
                || entry.path().extension().and_then(|ext| ext.to_str()) != Some("json")
            {
                continue;
            }
            let raw = match fs::read_to_string(entry.path()) {
                Ok(raw) => raw,
                Err(_) => continue,
            };
            let stored = match serde_json::from_str::<StoredAccount>(&raw) {
                Ok(item)
                    if validate_record_id(&item.record_id).is_ok()
                        && parse_payload_metadata(&item.payload_json).is_ok() =>
                {
                    item
                }
                Ok(_) | Err(_) => {
                    crate::runtime::append_log(
                        &self.paths,
                        "accounts.snapshot.unreadable",
                        serde_json::json!({
                            "file": entry.file_name().to_string_lossy(),
                        }),
                    );
                    continue;
                }
            };
            if !seen.insert(stored.record_id.clone()) {
                continue;
            }
            let active = match live_record_id {
                Some(live_id) => live_id == stored.record_id,
                None => expected_norm.as_deref().is_some_and(|expected| {
                    stored
                        .email
                        .as_deref()
                        .is_some_and(|email| email.eq_ignore_ascii_case(expected))
                }),
            };
            accounts.push(stored.summary(active));
        }

        accounts.sort_by(|left, right| {
            right
                .active
                .cmp(&left.active)
                .then_with(|| right.last_used_at.cmp(&left.last_used_at))
                .then_with(|| {
                    left.email
                        .as_deref()
                        .unwrap_or(&left.record_id)
                        .cmp(right.email.as_deref().unwrap_or(&right.record_id))
                })
        });
        Ok(accounts)
    }

    fn store_unlocked(&self, account: &StoredAccount) -> Result<()> {
        validate_record_id(&account.record_id)?;
        let bytes = serde_json::to_vec_pretty(account)
            .map_err(|e| AccountError::Storage(format!("Failed to serialize account: {e}")))?;
        atomic_write_private(&self.account_path(&account.record_id), &bytes)
            .map_err(AccountError::Storage)
    }

    fn load_optional_unlocked(&self, record_id: &str) -> Result<Option<StoredAccount>> {
        let path = self.account_path(record_id);
        match fs::read_to_string(&path) {
            Ok(raw) => Ok(serde_json::from_str(&raw).ok()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn load_unlocked(&self, record_id: &str) -> Result<StoredAccount> {
        let path = self.account_path(record_id);
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(AccountError::NotFound);
            }
            Err(error) => return Err(error.into()),
        };
        serde_json::from_str::<StoredAccount>(&raw)
            .map_err(|_| AccountError::Storage("Saved account file is corrupted".into()))
    }

    fn account_path(&self, record_id: &str) -> std::path::PathBuf {
        self.paths.accounts_dir().join(format!("{record_id}.json"))
    }

    fn lock_vault(&self) -> Result<VaultLock> {
        ensure_dir(&self.paths.accounts_dir())?;
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.paths.vault_lock_path())?;
        file.lock_exclusive()?;
        Ok(VaultLock { _file: file })
    }
}

struct VaultLock {
    _file: File,
}

fn ensure_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

pub fn validate_record_id(record_id: &str) -> Result<()> {
    if record_id.len() == 64
        && record_id
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(AccountError::InvalidRecordId)
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeStore(Mutex<Option<String>>);
    impl FakeStore {
        fn new(initial: Option<&str>) -> Self {
            Self(Mutex::new(initial.map(str::to_owned)))
        }
        fn value(&self) -> Option<String> {
            self.0.lock().unwrap().clone()
        }
    }
    impl SystemCredentialStore for FakeStore {
        fn write(&self, payload_json: &str) -> std::result::Result<(), String> {
            *self.0.lock().unwrap() = Some(payload_json.to_string());
            Ok(())
        }
        fn read(&self) -> std::result::Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn delete(&self) -> std::result::Result<(), String> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
        fn has_entry_without_prompt(&self) -> bool {
            self.0.lock().unwrap().is_some()
        }
    }

    fn make_payload(sub: &str, email: &str, name: &str, rt: &str) -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256"}"#);
        let claims = URL_SAFE_NO_PAD
            .encode(serde_json::json!({"sub": sub, "email": email, "name": name}).to_string());
        serde_json::json!({
            "auth_method": "consumer",
            "id_token": format!("{header}.{claims}.sig"),
            "token": {
                "access_token": format!("ya29.{sub}"),
                "token_type": "Bearer",
                "refresh_token": rt,
                "expiry": "2026-11-26T16:00:00.000000Z"
            }
        })
        .to_string()
    }

    #[test]
    fn snapshots_oauth_account_saves_rt_account_switches_and_protects_active_delete() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            home_dir: dir.path().join("home"),
            state_dir: dir.path().join("state"),
        };
        let vault = AccountVault::new(paths.clone());

        let oauth_payload = make_payload("1001", "oauth@gmail.com", "OAuth User", "1//oauth-rt");
        let store = FakeStore::new(Some(&oauth_payload));
        atomic_write_private(&paths.desktop_token_path(), oauth_payload.as_bytes()).unwrap();

        let listed = vault
            .list_accounts(&store, Some("oauth@gmail.com"))
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].active);
        assert_eq!(listed[0].source, AccountSource::OAuth);
        assert_eq!(listed[0].email.as_deref(), Some("oauth@gmail.com"));
        let oauth_id = listed[0].record_id.clone();

        // Save an RT account.
        let rt_payload = make_payload("2002", "rt@gmail.com", "RT User", "1//rt-token");
        let rt_meta = parse_payload_metadata(&rt_payload).unwrap();
        let rt_summary = vault
            .save_rt_account(
                &store,
                ExchangedCredential {
                    record_id: rt_meta.record_id.clone(),
                    subject: rt_meta.subject,
                    email: rt_meta.email,
                    name: rt_meta.name,
                    payload_json: rt_payload.clone(),
                },
            )
            .unwrap();
        assert_eq!(rt_summary.source, AccountSource::Rt);
        assert!(!rt_summary.active);

        // Cannot delete active OAuth account.
        assert!(matches!(
            vault.delete_account(&store, &oauth_id, Some("oauth@gmail.com")),
            Err(AccountError::ActiveAccount)
        ));

        // Switch to RT account; all 3 live stores update and OAuth stays in vault.
        let switched = vault
            .switch_after_shutdown(&store, &rt_summary.record_id)
            .unwrap();
        assert!(switched.active);
        assert_eq!(store.value().as_deref(), Some(rt_payload.as_str()));
        assert_eq!(
            fs::read_to_string(paths.cli_token_path()).unwrap(),
            rt_payload
        );
        assert_eq!(
            fs::read_to_string(paths.desktop_token_path()).unwrap(),
            rt_payload
        );

        let after_switch = vault.list_accounts(&store, Some("rt@gmail.com")).unwrap();
        assert_eq!(after_switch.len(), 2);
        assert_eq!(after_switch[0].record_id, rt_summary.record_id);
        assert!(after_switch[0].active);
        assert_eq!(after_switch[0].source, AccountSource::Rt);
        assert!(!after_switch[1].active);

        // Now deleting the inactive OAuth account succeeds.
        vault
            .delete_account(&store, &oauth_id, Some("rt@gmail.com"))
            .unwrap();
        assert_eq!(
            vault
                .list_accounts(&store, Some("rt@gmail.com"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn begin_google_login_snapshots_live_before_clearing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            home_dir: dir.path().join("home"),
            state_dir: dir.path().join("state"),
        };
        let vault = AccountVault::new(paths.clone());
        let payload = make_payload("3003", "keep@gmail.com", "Keep Me", "1//keep");
        let store = FakeStore::new(Some(&payload));
        persist_live_credential(&paths, &payload, &store).unwrap();

        vault.begin_google_login_after_shutdown(&store).unwrap();
        assert!(store.value().is_none());
        assert!(!paths.desktop_token_path().exists());
        assert!(!paths.cli_token_path().exists());

        let saved = vault.list_accounts(&store, None).unwrap();
        assert_eq!(saved.len(), 1);
        assert!(!saved[0].active);
        assert_eq!(saved[0].email.as_deref(), Some("keep@gmail.com"));
    }
}
