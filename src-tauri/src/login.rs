use serde::{Deserialize, Serialize};
use std::io::Write;
use std::{path::Path, sync::Mutex, time::Duration};
const CLIENT_ID: &str = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "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";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

pub static LOGIN_LOCK: Mutex<()> = Mutex::new(());

#[derive(Deserialize)]
pub struct TokenResponse {
    access_token: String,
    expires_in: i64,
    refresh_token: Option<String>,
    id_token: Option<String>,
}

#[derive(Serialize)]
pub struct LoginResult {
    pub email: Option<String>,
}

pub fn validate_rt(rt: &str) -> Result<&str, String> {
    let rt = rt.trim();
    if rt.is_empty() || rt.len() > 16384 || rt.chars().any(char::is_whitespace) {
        return Err("请输入单个有效的 Antigravity Refresh Token".into());
    }
    Ok(rt)
}

pub async fn exchange(
    client: &reqwest::Client,
    url: &str,
    rt: &str,
) -> Result<TokenResponse, String> {
    let response = client
        .post(url)
        .form(&[
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("refresh_token", rt),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|_| "无法连接 Google 登录服务，请检查网络后重试".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Google 拒绝登录（HTTP {}），请确认 RT 有效且属于 Antigravity",
            response.status().as_u16()
        ));
    }
    let token: TokenResponse = response
        .json()
        .await
        .map_err(|_| "Google 登录响应格式无效".to_string())?;
    if token.access_token.trim().is_empty() || !(1..=604800).contains(&token.expires_in) {
        return Err("Google 返回的登录凭据无效".into());
    }
    Ok(token)
}

pub fn payload(token: TokenResponse, rt: &str) -> Result<String, String> {
    let mut value = serde_json::json!({
        "token": {
            "access_token": token.access_token,
            "token_type": "Bearer",
            "refresh_token": token.refresh_token.filter(|v| !v.trim().is_empty()).unwrap_or_else(|| rt.into()),
            "expiry": (chrono::Utc::now() + chrono::Duration::seconds(token.expires_in)).to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
        },
        "auth_method": "consumer"
    });
    if let Some(id) = token.id_token {
        value["id_token"] = id.into();
    }
    serde_json::to_string(&value).map_err(|_| "生成登录凭据失败".into())
}

pub fn write_secret(path: &Path, secret: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("凭据路径无效")?;
    std::fs::create_dir_all(parent).map_err(|_| "创建 agy 凭据目录失败".to_string())?;
    let mut file = tempfile::Builder::new()
        .prefix(".agy-login-")
        .tempfile_in(parent)
        .map_err(|_| "创建凭据临时文件失败".to_string())?;
    // tempfile creates private 0600 files on Unix; persistence atomically replaces the target.
    file.write_all(secret.as_bytes())
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| "写入凭据失败".to_string())?;
    file.persist(path)
        .map_err(|_| format!("替换凭据失败：{}", path.display()))?;
    Ok(())
}

pub fn persist_with(
    home: &Path,
    secret: &str,
    write_system: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    write_system(secret)?;
    let gemini = home.join(".gemini");
    write_secret(
        &gemini.join("antigravity-cli/antigravity-oauth-token"),
        secret,
    )
    .map_err(|e| format!("系统凭据已更新，但 agy CLI 文件写入失败，请重试：{e}"))?;
    write_secret(&gemini.join("jetski-standalone-oauth-token"), secret)
        .map_err(|e| format!("agy CLI 凭据已更新，但桌面回退凭据写入失败，请重试：{e}"))?;
    Ok(())
}

pub async fn login(rt: String) -> Result<LoginResult, String> {
    let rt = validate_rt(&rt)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "初始化网络失败".to_string())?;
    let token = exchange(&client, TOKEN_URL, rt).await?;
    // Identity lookup is optional: valid tokens can still be installed if userinfo is unavailable.
    let email = match client
        .get("https://www.googleapis.com/oauth2/v2/userinfo")
        .bearer_auth(&token.access_token)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v["email"].as_str().map(str::to_owned)),
        _ => None,
    };
    let secret = payload(token, rt)?;
    tokio::task::spawn_blocking(move || {
        let _lock = LOGIN_LOCK
            .try_lock()
            .map_err(|_| "另一个登录正在写入，请稍后重试".to_string())?;
        let home = dirs::home_dir().ok_or("无法获取用户目录")?;
        persist_with(
            &home,
            &secret,
            crate::credential::write_antigravity_system_payload,
        )
    })
    .await
    .map_err(|_| "登录写入任务失败".to_string())??;
    Ok(LoginResult { email })
}

#[cfg(test)]
mod tests;
