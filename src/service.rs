use std::sync::Arc;

use anyhow::Context as _;
use serde_json::{json, Value};

use crate::accounts::AccountVault;
use crate::credential::default_system_store;
use crate::oauth;
use crate::paths::AppPaths;

#[derive(Clone)]
pub struct BridgeService {
    paths: Arc<AppPaths>,
}

impl BridgeService {
    pub fn new(paths: AppPaths) -> Self {
        Self {
            paths: Arc::new(paths),
        }
    }

    pub fn paths(&self) -> &AppPaths {
        self.paths.as_ref()
    }

    pub async fn handle(&self, path: &str, payload: Value) -> Value {
        let result = match path {
            "/accounts/list" => self.list(payload).await,
            "/accounts/rt-login" => self.rt_login(payload).await,
            "/accounts/switch" => self.switch(payload).await,
            "/accounts/delete" => self.delete(payload).await,
            "/accounts/google-login" => self.google_login().await,
            _ => Err(anyhow::anyhow!("Unknown AGY Plus request: {path}")),
        };
        match result {
            Ok(value) => value,
            Err(error) => {
                crate::runtime::append_log(
                    self.paths(),
                    "bridge.request.failed",
                    json!({"path": path, "message": error.to_string()}),
                );
                json!({"status": "failed", "message": error.to_string()})
            }
        }
    }

    async fn list(&self, payload: Value) -> anyhow::Result<Value> {
        let active_email = payload
            .get("activeEmail")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);
        let paths = self.paths.as_ref().clone();
        let accounts = tokio::task::spawn_blocking(move || {
            let store = default_system_store();
            AccountVault::new(paths)
                .list_accounts(store.as_ref(), active_email.as_deref())
                .map_err(anyhow::Error::from)
        })
        .await
        .context("Account listing task failed")??;
        Ok(json!({"status": "ok", "accounts": accounts}))
    }

    async fn rt_login(&self, payload: Value) -> anyhow::Result<Value> {
        let rt = payload
            .get("refreshToken")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Enter a valid Antigravity Refresh Token."))?
            .to_string();
        let switch_now = payload
            .get("switchNow")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let exchanged = oauth::exchange_and_identify(&rt)
            .await
            .map_err(|message| anyhow::anyhow!(message))?;
        let record_id = exchanged.record_id.clone();

        let paths = self.paths.as_ref().clone();
        let account = tokio::task::spawn_blocking(move || {
            let store = default_system_store();
            AccountVault::new(paths)
                .save_rt_account(store.as_ref(), exchanged)
                .map_err(anyhow::Error::from)
        })
        .await
        .context("Failed to save RT account")??;

        if switch_now {
            crate::runtime::queue_switch(self.paths(), &record_id)?;
            return Ok(json!({
                "status": "ok",
                "account": account,
                "restarting": true,
                "message": "Refresh Token verified. Restarting Antigravity with this account..."
            }));
        }

        Ok(json!({
            "status": "ok",
            "account": account,
            "restarting": false,
            "message": "Account saved. Click Switch to activate it."
        }))
    }

    async fn switch(&self, payload: Value) -> anyhow::Result<Value> {
        let record_id = required_record_id(&payload)?;
        let paths = self.paths.as_ref().clone();
        let validate_id = record_id.clone();
        tokio::task::spawn_blocking(move || {
            AccountVault::new(paths)
                .validate_switch(&validate_id)
                .map_err(anyhow::Error::from)
        })
        .await
        .context("Account switch validation failed")??;

        crate::runtime::queue_switch(self.paths(), &record_id)?;
        Ok(json!({
            "status": "ok",
            "restarting": true,
            "message": "Switching account and restarting Antigravity..."
        }))
    }

    async fn delete(&self, payload: Value) -> anyhow::Result<Value> {
        let record_id = required_record_id(&payload)?;
        let active_email = payload
            .get("activeEmail")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);
        let paths = self.paths.as_ref().clone();
        tokio::task::spawn_blocking(move || {
            let store = default_system_store();
            AccountVault::new(paths)
                .delete_account(store.as_ref(), &record_id, active_email.as_deref())
                .map_err(anyhow::Error::from)
        })
        .await
        .context("Account deletion task failed")??;
        Ok(json!({"status": "ok", "message": "Removed saved account."}))
    }

    async fn google_login(&self) -> anyhow::Result<Value> {
        crate::runtime::queue_google_login(self.paths())?;
        Ok(json!({
            "status": "ok",
            "restarting": true,
            "message": "Saving current account and restarting to Google sign-in..."
        }))
    }
}

fn required_record_id(payload: &Value) -> anyhow::Result<String> {
    let record_id = payload
        .get("recordId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing account identifier"))?;
    crate::accounts::validate_record_id(record_id)?;
    Ok(record_id.to_string())
}
