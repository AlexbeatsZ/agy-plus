use std::time::Duration;

use base64::Engine as _;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const CLIENT_ID: &str = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "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";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: i64,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UserInfo {
    pub id: Option<String>,
    pub email: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCredential {
    pub record_id: String,
    pub subject: Option<String>,
    pub email: Option<String>,
    pub name: Option<String>,
    pub auth_method: Option<String>,
    pub has_refresh_token: bool,
}

#[derive(Debug, Clone)]
pub struct ExchangedCredential {
    pub record_id: String,
    pub subject: Option<String>,
    pub email: Option<String>,
    pub name: Option<String>,
    pub payload_json: String,
}

pub fn validate_rt(rt: &str) -> Result<&str, String> {
    let rt = rt.trim();
    if rt.is_empty() || rt.len() > 16384 || rt.chars().any(char::is_whitespace) {
        return Err("Enter a single valid Antigravity Refresh Token.".into());
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
        .map_err(|_| {
            "Unable to reach Google sign-in service. Check your network connection and retry."
                .to_string()
        })?;
    if !response.status().is_success() {
        return Err(format!(
            "Google rejected sign-in (HTTP {}). Confirm the Refresh Token is valid for Antigravity.",
            response.status().as_u16()
        ));
    }
    let token: TokenResponse = response
        .json()
        .await
        .map_err(|_| "Google returned an invalid token response.".to_string())?;
    if token.access_token.trim().is_empty() || !(1..=604800).contains(&token.expires_in) {
        return Err("Google returned invalid credential fields.".into());
    }
    Ok(token)
}

pub async fn fetch_userinfo(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
) -> Option<UserInfo> {
    let response = client
        .get(url)
        .bearer_auth(access_token)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<UserInfo>().await.ok()
}

pub fn build_payload(token: TokenResponse, rt: &str) -> Result<String, String> {
    let refresh_token = token
        .refresh_token
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| rt.to_string());
    let mut value = serde_json::json!({
        "token": {
            "access_token": token.access_token,
            "token_type": "Bearer",
            "refresh_token": refresh_token,
            "expiry": (chrono::Utc::now() + chrono::Duration::seconds(token.expires_in))
                .to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        },
        "auth_method": "consumer",
    });
    if let Some(id_token) = token.id_token.filter(|v| !v.trim().is_empty()) {
        value["id_token"] = id_token.into();
    }
    serde_json::to_string(&value)
        .map_err(|_| "Failed to serialize OAuth credential payload.".into())
}

pub async fn exchange_and_identify(rt: &str) -> Result<ExchangedCredential, String> {
    exchange_and_identify_with_urls(rt, TOKEN_URL, USERINFO_URL).await
}

pub async fn exchange_and_identify_with_urls(
    rt: &str,
    token_url: &str,
    userinfo_url: &str,
) -> Result<ExchangedCredential, String> {
    let rt = validate_rt(rt)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Failed to initialize HTTP client.".to_string())?;
    let token = exchange(&client, token_url, rt).await?;
    let id_claims = token
        .id_token
        .as_deref()
        .and_then(decode_jwt_claims)
        .unwrap_or_default();
    let access_token = token.access_token.clone();
    let needs_userinfo =
        id_claims.email.is_none() || id_claims.name.is_none() || id_claims.subject.is_none();
    let userinfo = if needs_userinfo {
        fetch_userinfo(&client, userinfo_url, &access_token).await
    } else {
        None
    };

    let subject = id_claims
        .subject
        .or_else(|| userinfo.as_ref().and_then(|u| non_empty(u.id.as_deref())));
    let email = id_claims.email.or_else(|| {
        userinfo
            .as_ref()
            .and_then(|u| non_empty(u.email.as_deref()))
    });
    let name = id_claims
        .name
        .or_else(|| userinfo.as_ref().and_then(|u| non_empty(u.name.as_deref())));

    let final_rt = token
        .refresh_token
        .as_deref()
        .and_then(|v| non_empty(Some(v)))
        .unwrap_or_else(|| rt.to_string());
    let payload_json = build_payload(token, rt)?;
    let record_id = derive_record_id(
        subject.as_deref(),
        email.as_deref(),
        Some(final_rt.as_str()),
        Some(access_token.as_str()),
    )?;

    Ok(ExchangedCredential {
        record_id,
        subject,
        email,
        name,
        payload_json,
    })
}

pub fn parse_payload_metadata(payload_json: &str) -> Result<ParsedCredential, String> {
    let value: serde_json::Value =
        serde_json::from_str(payload_json).map_err(|_| "Credential payload is not valid JSON.")?;
    let object = value
        .as_object()
        .ok_or("Credential payload must be a JSON object.")?;
    let token = object
        .get("token")
        .and_then(serde_json::Value::as_object)
        .ok_or("Credential payload is missing token object.")?;
    let access_token = token
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .and_then(|s| non_empty(Some(s)));
    let refresh_token = token
        .get("refresh_token")
        .and_then(serde_json::Value::as_str)
        .and_then(|s| non_empty(Some(s)));
    if access_token.is_none() && refresh_token.is_none() {
        return Err("Credential payload has neither access_token nor refresh_token.".into());
    }

    let id_claims = object
        .get("id_token")
        .and_then(serde_json::Value::as_str)
        .and_then(decode_jwt_claims)
        .unwrap_or_default();
    let auth_method = object
        .get("auth_method")
        .and_then(serde_json::Value::as_str)
        .and_then(|s| non_empty(Some(s)));

    let record_id = derive_record_id(
        id_claims.subject.as_deref(),
        id_claims.email.as_deref(),
        refresh_token.as_deref(),
        access_token.as_deref(),
    )?;

    Ok(ParsedCredential {
        record_id,
        subject: id_claims.subject,
        email: id_claims.email,
        name: id_claims.name,
        auth_method,
        has_refresh_token: refresh_token.is_some(),
    })
}

pub fn derive_record_id(
    subject: Option<&str>,
    email: Option<&str>,
    refresh_token: Option<&str>,
    access_token: Option<&str>,
) -> Result<String, String> {
    let seed = if let Some(sub) = non_empty(subject) {
        format!("google-sub:{sub}")
    } else if let Some(email) = non_empty(email) {
        format!("google-email:{}", email.to_ascii_lowercase())
    } else if let Some(rt) = non_empty(refresh_token) {
        format!("google-rt:{rt}")
    } else if let Some(at) = non_empty(access_token) {
        format!("google-at:{at}")
    } else {
        return Err("Unable to derive account identity from credential.".into());
    };
    let digest = Sha256::digest(seed.as_bytes());
    Ok(format!("{digest:x}"))
}

#[derive(Default)]
struct JwtClaims {
    subject: Option<String>,
    email: Option<String>,
    name: Option<String>,
}

fn decode_jwt_claims(jwt: &str) -> Option<JwtClaims> {
    let segment = jwt.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(segment.trim_end_matches('='))
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    Some(JwtClaims {
        subject: non_empty(value.get("sub").and_then(serde_json::Value::as_str)),
        email: non_empty(value.get("email").and_then(serde_json::Value::as_str)),
        name: non_empty(value.get("name").and_then(serde_json::Value::as_str)),
    })
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_or_multiline_rt() {
        for rt in ["", "  ", "first\nsecond", "a b"] {
            assert!(validate_rt(rt).is_err());
        }
        assert_eq!(validate_rt("  example-rt  ").unwrap(), "example-rt");
    }

    #[test]
    fn preserves_rotated_refresh_and_id_tokens() {
        let token = TokenResponse {
            access_token: "access".into(),
            expires_in: 3600,
            refresh_token: Some("rotated".into()),
            id_token: Some("identity".into()),
        };
        let value: serde_json::Value =
            serde_json::from_str(&build_payload(token, "original").unwrap()).unwrap();
        assert_eq!(value["token"]["refresh_token"], "rotated");
        assert_eq!(value["id_token"], "identity");
        assert_eq!(value["auth_method"], "consumer");
        assert!(
            chrono::DateTime::parse_from_rfc3339(value["token"]["expiry"].as_str().unwrap())
                .is_ok()
        );
    }

    #[test]
    fn derives_identical_record_id_from_jwt_sub_or_userinfo_id() {
        let from_sub =
            derive_record_id(Some("123456"), Some("a@b.com"), Some("rt1"), None).unwrap();
        let from_sub_other_rt =
            derive_record_id(Some("123456"), Some("a@b.com"), Some("rt2"), None).unwrap();
        assert_eq!(from_sub, from_sub_other_rt);
        assert_eq!(from_sub.len(), 64);
    }

    #[tokio::test]
    async fn exchanges_token_fetches_userinfo_and_never_echoes_secret_on_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.ok",
                "expires_in": 3600,
                "refresh_token": "1//rotated"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/userinfo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "998877",
                "email": "alice@gmail.com",
                "name": "Alice"
            })))
            .mount(&server)
            .await;

        let exchanged = exchange_and_identify_with_urls(
            "1//original-rt",
            &format!("{}/token", server.uri()),
            &format!("{}/userinfo", server.uri()),
        )
        .await
        .unwrap();
        assert_eq!(exchanged.email.as_deref(), Some("alice@gmail.com"));
        assert_eq!(exchanged.name.as_deref(), Some("Alice"));
        assert_eq!(exchanged.subject.as_deref(), Some("998877"));
        let parsed: serde_json::Value = serde_json::from_str(&exchanged.payload_json).unwrap();
        assert_eq!(parsed["token"]["refresh_token"], "1//rotated");

        server.reset().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant",
                "secret": "1//secret-rt-leak"
            })))
            .mount(&server)
            .await;
        let error = exchange_and_identify_with_urls(
            "1//secret-rt-leak",
            &format!("{}/token", server.uri()),
            &format!("{}/userinfo", server.uri()),
        )
        .await
        .unwrap_err();
        assert!(error.contains("400"));
        assert!(!error.contains("1//secret-rt-leak"));
    }
}
