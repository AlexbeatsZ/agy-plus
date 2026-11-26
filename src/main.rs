#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod accounts;
mod cdp;
mod credential;
mod oauth;
mod paths;
mod process_timeout;
mod runtime;
mod service;

use anyhow::Context as _;
use serde_json::json;

use paths::AppPaths;
use service::BridgeService;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchMode {
    Version,
    ApplyPendingAction,
    WebLogin,
    AttachExisting(u16),
    LoginStartup,
    Explicit,
}

impl LaunchMode {
    fn from_args(args: &[String]) -> Self {
        if args.iter().any(|arg| arg == "--version" || arg == "-V") {
            Self::Version
        } else if args.iter().any(|arg| arg == "--apply-pending-action") {
            Self::ApplyPendingAction
        } else if args.iter().any(|arg| arg == "--web-login") {
            Self::WebLogin
        } else if let Some(port) = args.iter().find_map(|arg| {
            arg.strip_prefix("--attach-existing-port=")
                .and_then(|value| value.parse::<u16>().ok())
        }) {
            Self::AttachExisting(port)
        } else if args.iter().any(|arg| arg == "--attach-existing") {
            Self::AttachExisting(runtime::DEFAULT_DEBUG_PORT)
        } else if args.iter().any(|arg| arg == "--login-startup") {
            Self::LoginStartup
        } else {
            Self::Explicit
        }
    }
}

struct ManagedSession {
    process: Option<runtime::AppProcess>,
    debug_port: u16,
}

#[tokio::main]
async fn main() {
    if let Err(error) = main_impl().await {
        if let Ok(paths) = AppPaths::discover() {
            runtime::append_log(
                &paths,
                "launcher.fatal",
                json!({"message": format!("{error:#}")}),
            );
        }
        show_error(&format!("AGY Plus failed to start\n\n{error:#}"));
        std::process::exit(1);
    }
}

async fn main_impl() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mode = LaunchMode::from_args(&args);
    if mode == LaunchMode::Version {
        println!("agy-plus {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let paths = AppPaths::discover()?;
    #[cfg(target_os = "macos")]
    runtime::warn_if_code_signature_is_unstable(&paths);

    match mode {
        LaunchMode::Version => unreachable!(),
        LaunchMode::ApplyPendingAction => runtime::apply_pending_action(&paths),
        LaunchMode::WebLogin => run_managed(paths, "launcher.google_login_start", true).await,
        LaunchMode::AttachExisting(port) => run_attach_existing(paths, port).await,
        LaunchMode::LoginStartup => run_login_startup(paths).await,
        LaunchMode::Explicit => run_managed(paths, "launcher.explicit_start", false).await,
    }
}

async fn run_attach_existing(paths: AppPaths, debug_port: u16) -> anyhow::Result<()> {
    let _guard = runtime::acquire_launcher_guard(&paths)?;
    runtime::append_log(
        &paths,
        "launcher.attach_existing",
        json!({"port": debug_port}),
    );
    let session = ManagedSession {
        process: runtime::observe_antigravity_on_port(debug_port),
        debug_port,
    };
    let service = BridgeService::new(paths.clone());
    run_resident_supervisor(paths, service, session).await
}

async fn run_login_startup(paths: AppPaths) -> anyhow::Result<()> {
    let _guard = match runtime::acquire_launcher_guard(&paths) {
        Ok(guard) => guard,
        Err(error) => {
            runtime::append_log(
                &paths,
                "launcher.login_startup.skipped",
                json!({"reason": "launcherAlreadyRunning", "message": error.to_string()}),
            );
            return Ok(());
        }
    };
    runtime::append_log(&paths, "launcher.login_startup.waiting", json!({}));
    let discovered = runtime::wait_until_antigravity_starts().await;
    runtime::append_log(&paths, "launcher.login_startup.detected", json!({}));
    let mut pending = Some(discovered);
    let session = loop {
        let discovered = match pending.take() {
            Some(found) => found,
            None => runtime::wait_until_antigravity_starts().await,
        };
        if let Some(session) = take_over(&paths, discovered).await {
            break session;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    };
    let service = BridgeService::new(paths.clone());
    run_resident_supervisor(paths, service, session).await
}

async fn run_managed(
    paths: AppPaths,
    start_event: &str,
    capture_google_login: bool,
) -> anyhow::Result<()> {
    let target = runtime::resolve_antigravity_target()?;
    runtime::append_log(
        &paths,
        start_event,
        json!({"phases": runtime::START_PHASES}),
    );
    let stop_paths = paths.clone();
    tokio::task::spawn_blocking(move || runtime::stop_existing_runtime(&stop_paths))
        .await
        .context("Runtime stop task failed")??;
    let _guard = runtime::acquire_launcher_guard(&paths)?;
    let session = launch_session(&paths, &target, "launcher.antigravity_started").await?;
    if capture_google_login {
        tokio::spawn(runtime::capture_google_login(paths.clone()));
    }
    let service = BridgeService::new(paths.clone());
    run_resident_supervisor(paths, service, session).await
}

async fn launch_session(
    paths: &AppPaths,
    target: &runtime::AntigravityTarget,
    started_event: &str,
) -> anyhow::Result<ManagedSession> {
    let selection = runtime::select_debug_port(runtime::DEFAULT_DEBUG_PORT).await?;
    runtime::append_log(
        paths,
        "launcher.debug_port.selected",
        json!({
            "preferredPort": runtime::DEFAULT_DEBUG_PORT,
            "selectedPort": selection.port,
            "fallback": selection.preferred_bind_error.is_some(),
            "attempts": selection.preferred_bind_attempts,
            "preferredBindError": selection.preferred_bind_error,
        }),
    );
    let process = runtime::launch_antigravity(target, selection.port)?;
    runtime::append_log(
        paths,
        started_event,
        json!({
            "pid": process.id(),
            "debugPort": selection.port,
            "executable": target.executable,
        }),
    );
    Ok(ManagedSession {
        process: Some(process),
        debug_port: selection.port,
    })
}

async fn run_resident_supervisor(
    paths: AppPaths,
    service: BridgeService,
    mut session: ManagedSession,
) -> anyhow::Result<()> {
    loop {
        let port = session.debug_port;
        let process_pid = session.process.as_ref().map(runtime::AppProcess::id);
        let watchdog = tokio::spawn(cdp::run_watchdog(port, service.clone()));
        let stop = runtime::wait_until_antigravity_stops(session.process, port).await?;
        watchdog.abort();
        let _ = watchdog.await;

        let mut cdp_lost_pid = match stop {
            runtime::AppStop::Exited => {
                runtime::append_log(
                    &paths,
                    "launcher.antigravity.rearmed",
                    json!({"port": port}),
                );
                None
            }
            runtime::AppStop::CdpLost => {
                runtime::append_log(
                    &paths,
                    "launcher.antigravity.cdp_lost",
                    json!({"pid": process_pid, "port": port}),
                );
                process_pid
            }
        };

        let mut waiting_logged = false;
        session = loop {
            let discovered = runtime::wait_until_antigravity_starts().await;
            if cdp_lost_pid.is_some_and(|pid| discovered.process.id() == pid) {
                if let Some(port) = discovered.debug_port {
                    if cdp::endpoint_available(port).await {
                        break ManagedSession {
                            process: Some(discovered.process),
                            debug_port: port,
                        };
                    }
                }
                if !waiting_logged {
                    waiting_logged = true;
                    runtime::append_log(
                        &paths,
                        "launcher.antigravity.cdp_waiting",
                        json!({"pid": discovered.process.id()}),
                    );
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
            cdp_lost_pid = None;
            if let Some(next_session) = take_over(&paths, discovered).await {
                break next_session;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        };
    }
}

async fn take_over(paths: &AppPaths, discovered: runtime::DiscoveredApp) -> Option<ManagedSession> {
    if let Some(port) = discovered.debug_port {
        runtime::append_log(
            paths,
            "launcher.antigravity.attached",
            json!({"pid": discovered.process.id(), "debugPort": port}),
        );
        return Some(ManagedSession {
            process: Some(discovered.process),
            debug_port: port,
        });
    }

    let target = match runtime::resolve_antigravity_target() {
        Ok(target) => target,
        Err(error) => {
            runtime::append_log(
                paths,
                "launcher.antigravity.recovery_failed",
                json!({"phase": "resolve", "message": error.to_string()}),
            );
            return None;
        }
    };

    let stop_paths = paths.clone();
    match tokio::task::spawn_blocking(move || runtime::stop_antigravity_runtime(&stop_paths)).await
    {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            runtime::append_log(
                paths,
                "launcher.antigravity.recovery_failed",
                json!({"phase": "stop", "message": error.to_string()}),
            );
            return None;
        }
        Err(error) => {
            runtime::append_log(
                paths,
                "launcher.antigravity.recovery_failed",
                json!({"phase": "stop_join", "message": error.to_string()}),
            );
            return None;
        }
    }

    match launch_session(paths, &target, "launcher.antigravity.restarted").await {
        Ok(session) => Some(session),
        Err(error) => {
            runtime::append_log(
                paths,
                "launcher.antigravity.recovery_failed",
                json!({"phase": "launch", "message": error.to_string()}),
            );
            None
        }
    }
}

fn show_error(message: &str) {
    if std::env::var_os("AGY_PLUS_NO_DIALOG").is_some() {
        eprintln!("{message}");
        return;
    }

    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display alert \"AGY Plus\" message {} as critical",
            serde_json::to_string(message).unwrap_or_else(|_| "\"Startup failed\"".into())
        );
        let _ = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", &script])
            .output();
    }

    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt as _;
        use windows::core::PCWSTR;
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

        let title: Vec<u16> = OsStr::new("AGY Plus")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let body: Vec<u16> = OsStr::new(message)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(body.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_OK | MB_ICONERROR,
            );
        }
    }

    #[cfg(all(not(target_os = "macos"), not(windows)))]
    {
        eprintln!("{message}");
    }
}
