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
        serde_json::from_str(&payload(token, "original").unwrap()).unwrap();
    assert_eq!(value["token"]["refresh_token"], "rotated");
    assert_eq!(value["id_token"], "identity");
    assert_eq!(value["auth_method"], "consumer");
    assert!(
        chrono::DateTime::parse_from_rfc3339(value["token"]["expiry"].as_str().unwrap()).is_ok()
    );
}

#[test]
fn creates_cli_session_on_first_login_and_replaces_old_credentials() {
    let home = tempfile::tempdir().unwrap();
    let cli = home
        .path()
        .join(".gemini/antigravity-cli/antigravity-oauth-token");
    persist_with(home.path(), "old", |_| Ok(())).unwrap();
    persist_with(home.path(), "new", |s| {
        assert_eq!(s, "new");
        Ok(())
    })
    .unwrap();
    assert_eq!(std::fs::read_to_string(&cli).unwrap(), "new");
    assert_eq!(
        std::fs::read_to_string(home.path().join(".gemini/jetski-standalone-oauth-token")).unwrap(),
        "new"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(cli).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn system_failure_leaves_files_untouched_and_partial_write_reports_error() {
    let home = tempfile::tempdir().unwrap();
    assert!(persist_with(home.path(), "secret", |_| Err("denied".into())).is_err());
    assert!(!home.path().join(".gemini").exists());
    std::fs::create_dir_all(home.path().join(".gemini")).unwrap();
    std::fs::write(home.path().join(".gemini/antigravity-cli"), "blocking file").unwrap();
    assert!(persist_with(home.path(), "secret", |_| Ok(()))
        .unwrap_err()
        .contains("CLI"));
}

async fn mock_exchange(response: &'static str) -> Result<TokenResponse, String> {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/token", server.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = server.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = vec![];
        let mut buffer = [0u8; 4096];
        loop {
            let n = socket.read(&mut buffer).unwrap();
            request.extend_from_slice(&buffer[..n]);
            if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let len: usize = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                if request.len() >= end + 4 + len {
                    break;
                }
            }
        }
        let request = String::from_utf8(request).unwrap();
        assert!(request.contains("grant_type=refresh_token"));
        assert!(request.contains("refresh_token=test-rt"));
        socket.write_all(response.as_bytes()).unwrap();
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let result = exchange(&client, &url, "test-rt").await;
    worker.join().unwrap();
    result
}

#[tokio::test]
async fn exchanges_token_and_rejects_invalid_grant_without_echoing_secrets() {
    let token = mock_exchange("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"access_token\":\"access\",\"expires_in\":3600}").await.unwrap();
    assert_eq!(token.access_token, "access");
    let error = mock_exchange("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"error\":\"invalid_grant\",\"secret\":\"test-rt\"}").await.err().unwrap();
    assert!(error.contains("400"));
    assert!(!error.contains("test-rt"));
}
