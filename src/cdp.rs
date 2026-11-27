use std::collections::HashSet;
use std::time::Duration;

use anyhow::{bail, Context as _};
use futures_util::{SinkExt as _, StreamExt as _};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;

use crate::service::BridgeService;

const BINDING_NAME: &str = "agyPlusBridgeV1";
const BINDING_PLACEHOLDER: &str = "\"__AGY_PLUS_BINDING_NAME__\"";
const BRIDGE_SCRIPT: &str = include_str!("../assets/bridge.js");
const RENDERER_SCRIPT: &str = include_str!("../assets/renderer.js");

/// Lightweight heartbeat every 15s; confirmed-reply check detects macOS sleep/wake half-open sockets.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(45);
const LIVENESS_INTERVAL: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

const HEARTBEAT_SCRIPT: &str = "window.__agyPlus?.install?.() || null";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    pub url: String,
    pub web_socket_debugger_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BridgeRequest {
    id: String,
    path: String,
    #[serde(default)]
    payload: Value,
}

pub async fn endpoint_available(port: u16) -> bool {
    discover_targets(port)
        .await
        .is_ok_and(|targets| targets.iter().any(is_antigravity_hub_target))
}

/// Watches every local Antigravity Hub page target on `port` (supporting multiple windows
/// and language server restarts) and maintains live CDP injection sessions.
pub async fn run_watchdog(port: u16, service: BridgeService) -> anyhow::Result<()> {
    let mut attached = HashSet::<String>::new();
    let mut sessions = JoinSet::new();
    let mut waiting = WaitLog::default();
    let mut discover_attempts = 0_u64;

    loop {
        let delay = if attached.is_empty() && discover_attempts < 30 {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(3)
        };
        let sleep = tokio::time::sleep(delay);
        tokio::pin!(sleep);

        tokio::select! {
            _ = &mut sleep => {
                discover_attempts = discover_attempts.saturating_add(1);
                match discover_targets(port).await {
                    Ok(targets) => {
                        let hubs: Vec<Target> = targets
                            .into_iter()
                            .filter(is_antigravity_hub_target)
                            .collect();
                        if hubs.is_empty() && attached.is_empty() {
                            if let Some(attempts) = waiting.record("No Antigravity Hub page found yet") {
                                crate::runtime::append_log(
                                    service.paths(),
                                    "injection.target.waiting",
                                    json!({"message": "No Antigravity Hub page found yet", "attempts": attempts}),
                                );
                            }
                            continue;
                        }
                        if !hubs.is_empty() {
                            waiting.reset();
                        }
                        for target in hubs {
                            if attached.insert(target.id.clone()) {
                                crate::runtime::append_log(
                                    service.paths(),
                                    "injection.target.found",
                                    json!({"targetId": target.id, "title": target.title}),
                                );
                                let service = service.clone();
                                sessions.spawn(async move {
                                    let target_id = target.id.clone();
                                    let result = run_target_session(&target, service).await;
                                    (target_id, result)
                                });
                            }
                        }
                    }
                    Err(error) => {
                        if attached.is_empty() {
                            if let Some(attempts) = waiting.record(&error.to_string()) {
                                crate::runtime::append_log(
                                    service.paths(),
                                    "injection.target.waiting",
                                    json!({"message": error.to_string(), "attempts": attempts}),
                                );
                            }
                        }
                    }
                }
            }
            Some(joined) = sessions.join_next(), if !sessions.is_empty() => {
                match joined {
                    Ok((target_id, result)) => {
                        attached.remove(&target_id);
                        if let Err(error) = result {
                            crate::runtime::append_log(
                                service.paths(),
                                "injection.session.ended",
                                json!({"targetId": target_id, "message": error.to_string()}),
                            );
                        }
                    }
                    Err(join_error) => {
                        crate::runtime::append_log(
                            service.paths(),
                            "injection.session.panicked",
                            json!({"message": join_error.to_string()}),
                        );
                    }
                }
            }
        }
    }
}

pub fn is_antigravity_hub_target(target: &Target) -> bool {
    if target.kind != "page" || target.web_socket_debugger_url.is_none() {
        return false;
    }
    let url = target.url.as_str();
    let Some(rest) = url
        .strip_prefix("https://127.0.0.1:")
        .or_else(|| url.strip_prefix("http://127.0.0.1:"))
        .or_else(|| url.strip_prefix("https://localhost:"))
        .or_else(|| url.strip_prefix("http://localhost:"))
    else {
        return false;
    };
    let port_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let port_str = &rest[..port_end];
    if port_str.is_empty() || !port_str.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    // Exclude embedded devtools or static sub-pages that are not the Hub UI.
    !url.contains("/devtools/")
}

#[derive(Default)]
struct WaitLog {
    message: String,
    attempts: u64,
    logged_at: u64,
}

impl WaitLog {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn record(&mut self, message: &str) -> Option<u64> {
        self.attempts += 1;
        if message != self.message || self.attempts - self.logged_at >= 120 {
            self.message = message.to_string();
            self.logged_at = self.attempts;
            return Some(self.attempts);
        }
        None
    }
}

async fn discover_targets(port: u16) -> anyhow::Result<Vec<Target>> {
    local_client()?
        .get(format!("http://127.0.0.1:{port}/json"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .context("CDP endpoint is not ready yet")?
        .error_for_status()
        .context("CDP target list unavailable")?
        .json::<Vec<Target>>()
        .await
        .context("Invalid CDP target list JSON")
}

#[derive(Debug, Clone, Copy)]
struct SessionTuning {
    heartbeat: Duration,
    heartbeat_timeout: Duration,
    liveness: Duration,
}

impl Default for SessionTuning {
    fn default() -> Self {
        Self {
            heartbeat: HEARTBEAT_INTERVAL,
            heartbeat_timeout: HEARTBEAT_TIMEOUT,
            liveness: LIVENESS_INTERVAL,
        }
    }
}

async fn run_target_session(target: &Target, service: BridgeService) -> anyhow::Result<()> {
    run_target_session_with_tuning(target, service, SessionTuning::default()).await
}

async fn run_target_session_with_tuning(
    target: &Target,
    service: BridgeService,
    tuning: SessionTuning,
) -> anyhow::Result<()> {
    let websocket_url = target
        .web_socket_debugger_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Antigravity target missing WebSocket URL"))?;
    let (mut socket, _) = tokio_tungstenite::connect_async(websocket_url)
        .await
        .context("Failed to connect to Antigravity CDP WebSocket")?;
    let mut next_id = 1_u64;

    command(&mut socket, &mut next_id, "Runtime.enable", json!({})).await?;
    // Registration can acknowledge successfully while Page is disabled, yet
    // Chromium will not execute the preloads on the next navigation.
    command(&mut socket, &mut next_id, "Page.enable", json!({})).await?;
    let _ = command(
        &mut socket,
        &mut next_id,
        "Runtime.removeBinding",
        json!({"name": BINDING_NAME}),
    )
    .await;
    command(
        &mut socket,
        &mut next_id,
        "Runtime.addBinding",
        json!({"name": BINDING_NAME}),
    )
    .await?;

    // Clean up any previous document preload registrations before adding fresh ones.
    let previous = evaluate(
        &mut socket,
        &mut next_id,
        "Array.isArray(window.__agyPlusRegistrationIds) ? window.__agyPlusRegistrationIds : []",
    )
    .await
    .ok();
    if let Some(old_ids) = previous
        .as_ref()
        .and_then(|v| v.pointer("/result/result/value"))
        .and_then(Value::as_array)
    {
        for id in old_ids.iter().filter_map(Value::as_str) {
            let _ = command(
                &mut socket,
                &mut next_id,
                "Page.removeScriptToEvaluateOnNewDocument",
                json!({"identifier": id}),
            )
            .await;
        }
    }

    let bridge_script = browser_bridge_script();
    let mut new_registration_ids = Vec::new();
    let renderer_script = scoped_script(RENDERER_SCRIPT);
    for script in [&bridge_script, &renderer_script] {
        let registered = command(
            &mut socket,
            &mut next_id,
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source": script}),
        )
        .await?;
        if let Some(identifier) = registered
            .pointer("/result/identifier")
            .and_then(Value::as_str)
        {
            new_registration_ids.push(identifier.to_string());
        }
    }

    evaluate(&mut socket, &mut next_id, &bridge_script).await?;
    evaluate(&mut socket, &mut next_id, &renderer_script).await?;
    let reg_json = serde_json::to_string(&new_registration_ids)?;
    let _ = evaluate(
        &mut socket,
        &mut next_id,
        &format!("window.__agyPlusRegistrationIds = {reg_json};"),
    )
    .await;

    let health = evaluate(
        &mut socket,
        &mut next_id,
        "window.__agyPlus?.health?.() || null",
    )
    .await?;
    if !injection_healthy(&health) {
        bail!("Renderer injection health check failed: {health}");
    }
    crate::runtime::append_log(
        service.paths(),
        "injection.ready",
        json!({"targetId": target.id, "health": health.pointer("/result/result/value")}),
    );

    let (responses_tx, mut responses_rx) = mpsc::unbounded_channel::<(String, u64, Value)>();
    let mut heartbeat = tokio::time::interval(tuning.heartbeat);
    let mut liveness = tokio::time::interval(tuning.liveness);
    let mut pending_heartbeat: Option<u64> = None;
    let mut heartbeat_deadline = Instant::now() + tuning.heartbeat_timeout;

    loop {
        tokio::select! {
            next = socket.next() => {
                let Some(next) = next else { bail!("CDP WebSocket closed") };
                let message = next.context("Failed to read CDP message")?;
                let Message::Text(text) = message else { continue };
                let value: Value = serde_json::from_str(text.as_str()).context("Invalid CDP JSON message")?;

                if pending_heartbeat.is_some()
                    && value.get("id").and_then(Value::as_u64) == pending_heartbeat
                {
                    pending_heartbeat = None;
                    // A CDP acknowledgement only proves the socket works. A missing renderer,
                    // broken bridge or JS exception must end this session so the watchdog
                    // reinstalls both scripts and bindings in the current document.
                    if !injection_healthy(&value) {
                        crate::runtime::append_log(
                            service.paths(),
                            "injection.health.failed",
                            json!({"targetId": target.id}),
                        );
                        bail!("Renderer injection health lost; reconnecting");
                    }
                }

                let method = value.get("method").and_then(Value::as_str);
                if method == Some("Runtime.executionContextsCleared") {
                    pending_heartbeat = None;
                    heartbeat.reset_immediately();
                }
                if method == Some("Runtime.bindingCalled") {
                    let name = value.pointer("/params/name").and_then(Value::as_str);
                    if name != Some(BINDING_NAME) {
                        continue;
                    }
                    let raw = value
                        .pointer("/params/payload")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let Ok(request) = serde_json::from_str::<BridgeRequest>(raw) else {
                        continue;
                    };
                    let Some(context_id) = value.pointer("/params/executionContextId").and_then(Value::as_u64) else {
                        continue;
                    };
                    let tx = responses_tx.clone();
                    let service = service.clone();
                    tokio::spawn(async move {
                        let result = service.handle(&request.path, request.payload).await;
                        let _ = tx.send((request.id, context_id, result));
                    });
                } else if value.get("id").is_some()
                    && (value.get("error").is_some()
                        || value.pointer("/result/exceptionDetails").is_some())
                {
                    crate::runtime::append_log(
                        service.paths(),
                        "cdp.command.failed",
                        json!({"targetId": target.id, "response": value}),
                    );
                }
            }
            Some((request_id, context_id, value)) = responses_rx.recv() => {
                let expression = format!(
                    "window.__agyPlusBridgeResolve?.({},{})",
                    serde_json::to_string(&request_id)?,
                    serde_json::to_string(&value)?
                );
                send_evaluate_in_context(&mut socket, &mut next_id, &expression, Some(context_id)).await?;
            }
            _ = liveness.tick() => {
                if pending_heartbeat.is_some() && Instant::now() >= heartbeat_deadline {
                    crate::runtime::append_log(
                        service.paths(),
                        "injection.heartbeat.stalled",
                        json!({"targetId": target.id, "timeoutSeconds": tuning.heartbeat_timeout.as_secs()}),
                    );
                    bail!(
                        "CDP heartbeat stalled for more than {}s",
                        tuning.heartbeat_timeout.as_secs()
                    );
                }
            }
            _ = heartbeat.tick() => {
                if pending_heartbeat.is_none() {
                    let id = send_evaluate_async(&mut socket, &mut next_id, HEARTBEAT_SCRIPT).await?;
                    pending_heartbeat = Some(id);
                    heartbeat_deadline = Instant::now() + tuning.heartbeat_timeout;
                }
            }
        }
    }
}

pub fn browser_bridge_script() -> String {
    scoped_script(&BRIDGE_SCRIPT.replace(
        BINDING_PLACEHOLDER,
        &serde_json::to_string(BINDING_NAME).expect("binding name is serializable"),
    ))
}

fn scoped_script(source: &str) -> String {
    // Preloads also run in child frames and survive navigation. Only install in
    // the local Hub's top frame, never an OAuth page or embedded plugin.
    format!(
        "(() => {{ if (window !== window.top || !['http:', 'https:'].includes(location.protocol) || !['127.0.0.1', 'localhost'].includes(location.hostname) || !location.port) return; {source}\n }})()"
    )
}

fn injection_healthy(response: &Value) -> bool {
    if response.get("error").is_some() || response.pointer("/result/exceptionDetails").is_some() {
        return false;
    }
    let Some(health) = response.pointer("/result/result/value") else {
        return false;
    };
    health.get("observerReady").and_then(Value::as_bool) == Some(true)
        && health.get("bridgeReady").and_then(Value::as_bool) == Some(true)
        && (health.get("ready").and_then(Value::as_bool) == Some(true)
            || health.get("documentLoading").and_then(Value::as_bool) == Some(true))
}

async fn evaluate<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    next_id: &mut u64,
    expression: &str,
) -> anyhow::Result<Value>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    command(
        socket,
        next_id,
        "Runtime.evaluate",
        json!({"expression": expression, "returnByValue": true, "awaitPromise": true}),
    )
    .await
}

async fn command<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    next_id: &mut u64,
    method: &str,
    params: Value,
) -> anyhow::Result<Value>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let id = *next_id;
    *next_id += 1;
    write_message(
        socket,
        Message::Text(
            serde_json::to_string(&json!({"id": id, "method": method, "params": params}))?.into(),
        ),
    )
    .await?;
    loop {
        let message = tokio::time::timeout(Duration::from_secs(8), socket.next())
            .await
            .context("CDP command timed out")?
            .ok_or_else(|| anyhow::anyhow!("CDP WebSocket closed"))??;
        let Message::Text(text) = message else {
            continue;
        };
        let response: Value = serde_json::from_str(text.as_str())?;
        if response.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if let Some(error) = response.get("error") {
            bail!("CDP {method} failed: {error}");
        }
        if let Some(exception) = response.pointer("/result/exceptionDetails") {
            bail!("CDP {method} exception: {exception}");
        }
        return Ok(response);
    }
}

async fn send_evaluate_async<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    next_id: &mut u64,
    expression: &str,
) -> anyhow::Result<u64>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    send_evaluate_in_context(socket, next_id, expression, None).await
}

async fn send_evaluate_in_context<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    next_id: &mut u64,
    expression: &str,
    context_id: Option<u64>,
) -> anyhow::Result<u64>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let id = *next_id;
    *next_id += 1;
    let mut params = json!({"expression": expression, "returnByValue": true});
    if let Some(context_id) = context_id {
        params["contextId"] = json!(context_id);
    }
    write_message(
        socket,
        Message::Text(
            serde_json::to_string(&json!({
                "id": id,
                "method": "Runtime.evaluate",
                "params": params
            }))?
            .into(),
        ),
    )
    .await?;
    Ok(id)
}

async fn write_message<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    message: Message,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::time::timeout(WRITE_TIMEOUT, socket.send(message))
        .await
        .context("CDP write timed out")?
        .context("Failed to write CDP message")
}

fn local_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .context("Failed to create local CDP HTTP client")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppPaths;

    fn target(url: &str, kind: &str) -> Target {
        Target {
            id: url.to_string(),
            kind: kind.to_string(),
            title: "Jetski Web".to_string(),
            url: url.to_string(),
            web_socket_debugger_url: Some("ws://127.0.0.1/devtools/page/1".to_string()),
        }
    }

    #[test]
    fn hub_target_matches_loopback_pages_and_excludes_data_webviews_and_external() {
        assert!(is_antigravity_hub_target(&target(
            "https://127.0.0.1:54321/",
            "page"
        )));
        assert!(is_antigravity_hub_target(&target(
            "https://127.0.0.1:54321/settings?tab=account#t=abc",
            "page"
        )));
        assert!(is_antigravity_hub_target(&target(
            "http://127.0.0.1:8080/",
            "page"
        )));
        for (url, kind) in [
            ("data:text/html,curtain", "page"),
            ("devtools://devtools/bundled/inspector.html", "page"),
            ("https://google.com/", "page"),
            ("https://127.0.0.1:54321/", "webview"),
            ("https://127.0.0.1:54321/", "service_worker"),
            ("https://127.0.0.1/", "page"),
        ] {
            assert!(
                !is_antigravity_hub_target(&target(url, kind)),
                "unexpected match for {kind} {url}"
            );
        }
    }

    #[test]
    fn bridge_script_binds_name_and_contains_no_credentials() {
        let script = browser_bridge_script();
        assert!(script.contains(BINDING_NAME));
        assert!(!script.contains(BINDING_PLACEHOLDER));
        assert!(!script.contains("access_token"));
        assert!(!script.contains("refresh_token"));
    }

    #[tokio::test]
    async fn session_fails_when_heartbeat_stops_responding() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut evaluates = 0_u32;
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let value: Value = serde_json::from_str(text.as_str()).unwrap();
                let id = value.get("id").cloned().unwrap_or(Value::Null);
                let method = value.get("method").and_then(Value::as_str).unwrap_or("");
                if method == "Runtime.evaluate" {
                    evaluates += 1;
                    // Setup runs 5 evaluates (previous reg ids, bridge, renderer, store reg ids, health).
                    // Drop replies after setup to simulate sleep/wake half-open socket.
                    if evaluates > 5 {
                        continue;
                    }
                }
                let response = if method == "Page.addScriptToEvaluateOnNewDocument" {
                    json!({"id": id, "result": {"identifier": "reg-1"}})
                } else {
                    json!({
                        "id": id,
                        "result": {
                            "result": {
                                "type": "object",
                                "value": {"ready": true, "observerReady": true, "bridgeReady": true}
                            }
                        }
                    })
                };
                if socket
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let service = BridgeService::new(AppPaths {
            home_dir: dir.path().join("home"),
            state_dir: dir.path().join("state"),
        });
        let mut injected = target("https://127.0.0.1:54321/", "page");
        injected.web_socket_debugger_url = Some(format!("ws://127.0.0.1:{port}/devtools/page/1"));
        let tuning = SessionTuning {
            heartbeat: Duration::from_millis(50),
            heartbeat_timeout: Duration::from_millis(200),
            liveness: Duration::from_millis(50),
        };

        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_target_session_with_tuning(&injected, service, tuning),
        )
        .await
        .expect("session must terminate instead of hanging")
        .expect_err("stalled heartbeat must return an error");
        assert!(result.to_string().contains("heartbeat"), "{result}");
        server.abort();
    }

    #[tokio::test]
    async fn session_dispatches_binding_call_and_resolves_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut sent_call = false;
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let req: Value = serde_json::from_str(text.as_str()).unwrap();
                let id = req.get("id").cloned().unwrap_or(Value::Null);
                let method = req.get("method").and_then(Value::as_str).unwrap_or("");
                let expr = req
                    .pointer("/params/expression")
                    .and_then(Value::as_str)
                    .unwrap_or("");

                if expr.starts_with("window.__agyPlusBridgeResolve") {
                    assert!(expr.contains("req-list-1"));
                    assert!(expr.contains("\"status\":\"ok\""));
                    assert_eq!(req.pointer("/params/contextId"), Some(&json!(42)));
                    socket.close(None).await.unwrap();
                    return;
                }

                let response = if method == "Page.addScriptToEvaluateOnNewDocument" {
                    json!({"id": id, "result": {"identifier": "reg-1"}})
                } else {
                    json!({
                        "id": id,
                        "result": {
                            "result": {
                                "type": "object",
                                "value": {"ready": true, "observerReady": true, "bridgeReady": true}
                            }
                        }
                    })
                };
                socket
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .unwrap();

                if !sent_call && expr == "window.__agyPlus?.health?.() || null" {
                    sent_call = true;
                    let binding = json!({
                        "method": "Runtime.bindingCalled",
                        "params": {
                            "name": BINDING_NAME,
                            "executionContextId": 42,
                            "payload": json!({"id": "req-list-1", "path": "/accounts/list", "payload": {}}).to_string()
                        }
                    });
                    socket
                        .send(Message::Text(binding.to_string().into()))
                        .await
                        .unwrap();
                }
            }
            panic!("binding response was never sent");
        });

        let dir = tempfile::tempdir().unwrap();
        let service = BridgeService::new(AppPaths {
            home_dir: dir.path().join("home"),
            state_dir: dir.path().join("state"),
        });
        let mut injected = target("https://127.0.0.1:54321/", "page");
        injected.web_socket_debugger_url = Some(format!("ws://127.0.0.1:{port}/devtools/page/1"));
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            run_target_session_with_tuning(
                &injected,
                service,
                SessionTuning {
                    heartbeat: Duration::from_secs(60),
                    heartbeat_timeout: Duration::from_secs(60),
                    liveness: Duration::from_secs(60),
                },
            ),
        )
        .await
        .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn acknowledged_but_unhealthy_heartbeat_ends_session_for_reinjection() {
        for unhealthy in [
            json!({"result": {"result": {"value": null}}}),
            json!({"result": {"result": {"value": {"ready": true, "observerReady": true, "bridgeReady": false}}}}),
            json!({"result": {"exceptionDetails": {"text": "Execution context was destroyed"}}}),
            json!({"error": {"code": -32000, "message": "Cannot find context"}}),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                while let Some(Ok(Message::Text(text))) = socket.next().await {
                    let request: Value = serde_json::from_str(text.as_str()).unwrap();
                    let id = request["id"].clone();
                    let mut response = if request
                        .pointer("/params/expression")
                        .and_then(Value::as_str)
                        == Some(HEARTBEAT_SCRIPT)
                    {
                        unhealthy.clone()
                    } else if request["method"] == "Page.addScriptToEvaluateOnNewDocument" {
                        json!({"result": {"identifier": "reg-1"}})
                    } else {
                        json!({"result": {"result": {"value": {"ready": true, "observerReady": true, "bridgeReady": true}}}})
                    };
                    response["id"] = id;
                    if socket
                        .send(Message::Text(response.to_string().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });
            let dir = tempfile::tempdir().unwrap();
            let service = BridgeService::new(AppPaths {
                home_dir: dir.path().join("home"),
                state_dir: dir.path().join("state"),
            });
            let mut injected = target("https://127.0.0.1:54321/", "page");
            injected.web_socket_debugger_url =
                Some(format!("ws://127.0.0.1:{port}/devtools/page/1"));
            let error = tokio::time::timeout(
                Duration::from_secs(5),
                run_target_session_with_tuning(&injected, service, SessionTuning::default()),
            )
            .await
            .unwrap()
            .expect_err("unhealthy acknowledgement must trigger reconnect");
            assert!(error.to_string().contains("health lost"), "{error}");
            server.abort();
        }
    }

    #[test]
    fn parser_startup_is_healthy_only_with_an_installed_observer_and_bridge() {
        let response = json!({"result": {"result": {"value": {
            "ready": false, "observerReady": true, "bridgeReady": true, "documentLoading": true
        }}}});
        assert!(injection_healthy(&response));
        let mut missing_observer = response.clone();
        missing_observer["result"]["result"]["value"]["observerReady"] = json!(false);
        assert!(!injection_healthy(&missing_observer));
        let mut ready_document = response;
        ready_document["result"]["result"]["value"]["documentLoading"] = json!(false);
        assert!(!injection_healthy(&ready_document));
    }
}
