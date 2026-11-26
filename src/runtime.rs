use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context as _};
use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use serde_json::json;
#[cfg(unix)]
use sysinfo::Signal;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::accounts::AccountVault;
use crate::credential::{capture_live_backup, default_system_store, restore_live_backup};
use crate::paths::{atomic_write_private, AppPaths};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub const DEFAULT_DEBUG_PORT: u16 = 9239;
pub const START_PHASES: [&str; 4] = ["stop", "lock", "launch", "inject"];

const DEBUG_PORT_BIND_ATTEMPTS: u32 = 15;
const DEBUG_PORT_BIND_INTERVAL: Duration = Duration::from_millis(200);
const LOG_ROTATE_BYTES: u64 = 8 * 1024 * 1024;
const CDP_LOST_AFTER: Duration = Duration::from_secs(120);
const CDP_PROBE_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PendingAction {
    SwitchAccount {
        #[serde(rename = "recordId")]
        record_id: String,
    },
    GoogleLogin,
}

pub struct LauncherGuard {
    _file: File,
}

#[derive(Debug)]
pub struct DebugPortSelection {
    pub port: u16,
    pub preferred_bind_error: Option<String>,
    pub preferred_bind_attempts: u32,
}

#[derive(Debug, Clone)]
pub struct AntigravityTarget {
    pub executable: PathBuf,
    pub install_root: PathBuf,
}

pub enum AppProcess {
    Child(Child),
    Observed {
        pid: Pid,
        start_time: u64,
        system: System,
    },
}

impl AppProcess {
    pub fn id(&self) -> u32 {
        match self {
            Self::Child(child) => child.id(),
            Self::Observed { pid, .. } => pid.as_u32(),
        }
    }

    pub fn is_running(&mut self) -> anyhow::Result<bool> {
        match self {
            Self::Child(child) => Ok(child.try_wait()?.is_none()),
            Self::Observed {
                pid,
                start_time,
                system,
            } => {
                system.refresh_processes_specifics(
                    ProcessesToUpdate::Some(&[*pid]),
                    true,
                    ProcessRefreshKind::nothing(),
                );
                Ok(system
                    .process(*pid)
                    .is_some_and(|process| process.start_time() == *start_time))
            }
        }
    }
}

pub struct DiscoveredApp {
    pub process: AppProcess,
    pub debug_port: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppStop {
    Exited,
    CdpLost,
}

pub fn append_log(paths: &AppPaths, event: &str, details: serde_json::Value) {
    let _ = fs::create_dir_all(&paths.state_dir);
    let path = paths.log_path();
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > LOG_ROTATE_BYTES) {
        let _ = fs::rename(&path, path.with_extension("log.1"));
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let value = json!({
        "timestamp": timestamp,
        "event": sanitize_event(event),
        "details": details,
    });
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{value}");
    }
}

pub fn acquire_launcher_guard(paths: &AppPaths) -> anyhow::Result<LauncherGuard> {
    fs::create_dir_all(&paths.state_dir)?;
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(paths.launcher_lock_path())?;
    file.try_lock_exclusive()
        .context("Another agy-plus launcher is already running")?;
    Ok(LauncherGuard { _file: file })
}

pub async fn select_debug_port(preferred_port: u16) -> anyhow::Result<DebugPortSelection> {
    let mut last_error = None;
    let mut attempts = 0_u32;
    for attempt in 1..=DEBUG_PORT_BIND_ATTEMPTS {
        attempts = attempt;
        match TcpListener::bind((Ipv4Addr::LOCALHOST, preferred_port)) {
            Ok(listener) => {
                let port = listener.local_addr()?.port();
                drop(listener);
                return Ok(DebugPortSelection {
                    port,
                    preferred_bind_error: None,
                    preferred_bind_attempts: attempts,
                });
            }
            Err(error) => {
                let retryable = error.kind() == std::io::ErrorKind::AddrInUse;
                last_error = Some(error);
                if !retryable || attempt == DEBUG_PORT_BIND_ATTEMPTS {
                    break;
                }
                tokio::time::sleep(DEBUG_PORT_BIND_INTERVAL).await;
            }
        }
    }
    let error = last_error.expect("select_debug_port must attempt binding at least once");
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .context("Unable to allocate a fallback local port for Antigravity CDP")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(DebugPortSelection {
        port,
        preferred_bind_error: Some(error.to_string()),
        preferred_bind_attempts: attempts,
    })
}

#[cfg(target_os = "macos")]
pub fn warn_if_code_signature_is_unstable(paths: &AppPaths) {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let Ok(output) = Command::new("/usr/bin/codesign")
        .args(["-d", "-r-"])
        .arg(&executable)
        .output()
    else {
        return;
    };
    let requirement = parse_designated_requirement(&String::from_utf8_lossy(&output.stderr))
        .or_else(|| parse_designated_requirement(&String::from_utf8_lossy(&output.stdout)));
    let Some(requirement) = requirement else {
        return;
    };
    if requirement_uses_stable_identity(&requirement) {
        return;
    }
    append_log(
        paths,
        "launcher.signature.unstable",
        json!({
            "requirement": requirement,
            "hint": "Binary uses ad-hoc/linker signature. Run bash macos/sign.sh to sign with Local Development Code Signing.",
        }),
    );
}

#[cfg(target_os = "macos")]
fn parse_designated_requirement(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        line.trim()
            .trim_start_matches('#')
            .trim()
            .strip_prefix("designated =>")
            .map(|value| value.trim().to_string())
    })
}

#[cfg(target_os = "macos")]
fn requirement_uses_stable_identity(requirement: &str) -> bool {
    requirement.contains("certificate leaf")
}

pub fn resolve_antigravity_target() -> anyhow::Result<AntigravityTarget> {
    if let Some(explicit) = std::env::var_os("AGY_PLUS_APP_PATH").filter(|v| !v.is_empty()) {
        if let Some(target) = normalize_app_candidate(Path::new(&explicit)) {
            return Ok(target);
        }
        bail!(
            "AGY_PLUS_APP_PATH does not point to Antigravity executable: {}",
            Path::new(&explicit).display()
        );
    }

    #[cfg(target_os = "macos")]
    {
        let mut candidates = vec![PathBuf::from("/Applications/Antigravity.app")];
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("Applications").join("Antigravity.app"));
        }
        for candidate in candidates {
            if let Some(target) = normalize_app_candidate(&candidate) {
                return Ok(target);
            }
        }
    }

    #[cfg(windows)]
    {
        let mut candidates = Vec::new();
        if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            candidates.push(local.join("Programs").join("Antigravity"));
            candidates.push(local.join("Programs").join("antigravity"));
            candidates.push(local.join("Antigravity"));
        }
        if let Some(program_files) = std::env::var_os("ProgramFiles").map(PathBuf::from) {
            candidates.push(program_files.join("Antigravity"));
        }
        if let Some(program_files_x86) = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from) {
            candidates.push(program_files_x86.join("Antigravity"));
        }
        for candidate in candidates {
            if let Some(target) = normalize_app_candidate(&candidate) {
                return Ok(target);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let mut candidates = vec![
            PathBuf::from("/opt/Antigravity/antigravity"),
            PathBuf::from("/usr/bin/antigravity"),
        ];
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".local/opt/antigravity/antigravity"));
        }
        for candidate in candidates {
            if let Some(target) = normalize_app_candidate(&candidate) {
                return Ok(target);
            }
        }
    }

    bail!("Official Antigravity desktop application was not found")
}

pub fn launch_antigravity(
    target: &AntigravityTarget,
    debug_port: u16,
) -> anyhow::Result<AppProcess> {
    let mut command = background_command(&target.executable);
    command
        .arg(format!("--remote-debugging-port={debug_port}"))
        .arg(format!(
            "--remote-allow-origins=http://127.0.0.1:{debug_port}"
        ));
    if let Some(user_data_dir) =
        std::env::var_os("AGY_PLUS_USER_DATA_DIR").filter(|value| !value.is_empty())
    {
        command.arg(format!(
            "--user-data-dir={}",
            Path::new(&user_data_dir).display()
        ));
    }
    let child = command
        .spawn()
        .with_context(|| format!("Failed to launch {}", target.executable.display()))?;
    Ok(AppProcess::Child(child))
}

pub fn stop_existing_runtime(paths: &AppPaths) -> anyhow::Result<()> {
    stop_runtime_processes(paths, true)
}

pub fn stop_antigravity_runtime(paths: &AppPaths) -> anyhow::Result<()> {
    stop_runtime_processes(paths, false)
}

fn stop_runtime_processes(paths: &AppPaths, include_launchers: bool) -> anyhow::Result<()> {
    let current_pid = std::process::id();
    let current_exe =
        std::env::current_exe().context("Unable to locate current agy-plus binary")?;
    let current_key = normalized_path(&current_exe);
    let mut system = System::new();
    refresh_process_identity(&mut system, ProcessesToUpdate::All);

    let targets = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            if pid.as_u32() == current_pid {
                return None;
            }
            let same_launcher = process
                .exe()
                .is_some_and(|path| normalized_path(path) == current_key);
            let known_launcher = is_agy_plus_launcher(process.name());
            let antigravity = is_antigravity_bundle_process(process.name(), process.exe());
            (antigravity || (include_launchers && (same_launcher || known_launcher)))
                .then_some((*pid, process.name().to_string_lossy().to_string()))
        })
        .collect::<Vec<_>>();

    append_log(
        paths,
        "runtime.stop.started",
        json!({"processCount": targets.len(), "includeLaunchers": include_launchers}),
    );

    if targets.is_empty() {
        append_log(paths, "runtime.stop.completed", json!({}));
        return Ok(());
    }

    // Prefer graceful SIGTERM on Unix so Antigravity can flush SQLite WAL and stop language_server.
    #[allow(unused_mut)]
    let mut pids: Vec<Pid> = targets.iter().map(|(pid, _)| *pid).collect();
    #[cfg(unix)]
    {
        for pid in &pids {
            if let Some(process) = system.process(*pid) {
                let _ = process.kill_with(Signal::Term);
            }
        }
        for _ in 0..40 {
            if pids.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
            refresh_process_identity(&mut system, ProcessesToUpdate::Some(&pids));
            pids.retain(|pid| system.process(*pid).is_some());
        }
    }

    for pid in &pids {
        if let Some(process) = system.process(*pid) {
            let _ = process.kill();
        }
    }
    wait_for_pids(&mut system, pids)?;
    append_log(paths, "runtime.stop.completed", json!({}));
    Ok(())
}

pub fn queue_switch(paths: &AppPaths, record_id: &str) -> anyhow::Result<()> {
    queue_pending_action(
        paths,
        PendingAction::SwitchAccount {
            record_id: record_id.to_string(),
        },
    )
}

pub fn queue_google_login(paths: &AppPaths) -> anyhow::Result<()> {
    queue_pending_action(paths, PendingAction::GoogleLogin)
}

fn queue_pending_action(paths: &AppPaths, pending: PendingAction) -> anyhow::Result<()> {
    let encoded = serde_json::to_vec(&pending)?;
    atomic_write_private(&paths.pending_action_path(), &encoded)
        .map_err(|message| anyhow::anyhow!(message))?;
    let executable = std::env::current_exe().context("Unable to locate agy-plus binary")?;
    let mut command = background_command(executable);
    command.arg("--apply-pending-action");
    if let Err(error) = command.spawn() {
        let _ = fs::remove_file(paths.pending_action_path());
        return Err(error).context("Unable to spawn account action helper");
    }
    Ok(())
}

pub fn apply_pending_action(paths: &AppPaths) -> anyhow::Result<()> {
    let bytes = fs::read(paths.pending_action_path()).context("No pending account action found")?;
    let action: PendingAction =
        serde_json::from_slice(&bytes).context("Pending account action is invalid")?;
    let vault = AccountVault::new(paths.clone());
    if let PendingAction::SwitchAccount { record_id } = &action {
        vault.validate_switch(record_id)?;
    }

    // Give the injected UI brief time to receive the bridge acknowledgement before stopping Antigravity.
    std::thread::sleep(Duration::from_millis(350));
    let store = default_system_store();
    let backup = capture_live_backup(paths, store.as_ref(), false);
    stop_existing_runtime(paths)?;

    let action_result = match &action {
        PendingAction::SwitchAccount { record_id } => vault
            .switch_after_shutdown(store.as_ref(), record_id)
            .map(|_| ()),
        PendingAction::GoogleLogin => vault.begin_google_login_after_shutdown(store.as_ref()),
    };

    if let Err(error) = &action_result {
        let rollback = restore_live_backup(paths, &backup, store.as_ref());
        append_log(
            paths,
            match action {
                PendingAction::SwitchAccount { .. } => "account.switch.failed",
                PendingAction::GoogleLogin => "account.google_login.failed",
            },
            json!({"message": error.to_string(), "rollbackOk": rollback.is_ok()}),
        );
    } else {
        append_log(
            paths,
            match action {
                PendingAction::SwitchAccount { .. } => "account.switch.completed",
                PendingAction::GoogleLogin => "account.google_login.started",
            },
            json!({}),
        );
    }

    let _ = fs::remove_file(paths.pending_action_path());
    let launch_result = match action {
        PendingAction::SwitchAccount { .. } => spawn_normal_launcher(),
        PendingAction::GoogleLogin if action_result.is_ok() => spawn_web_login_launcher(),
        PendingAction::GoogleLogin => spawn_normal_launcher(),
    };

    if let Err(error) = launch_result {
        if matches!(action, PendingAction::GoogleLogin) && action_result.is_ok() {
            let _ = restore_live_backup(paths, &backup, store.as_ref());
            let _ = spawn_normal_launcher();
        }
        return Err(error);
    }
    action_result.map_err(Into::into)
}

pub fn spawn_normal_launcher() -> anyhow::Result<()> {
    let executable = std::env::current_exe().context("Unable to locate agy-plus binary")?;
    let mut command = background_command(executable);
    command.spawn().context("Failed to restart agy-plus")?;
    Ok(())
}

fn spawn_web_login_launcher() -> anyhow::Result<()> {
    let executable = std::env::current_exe().context("Unable to locate agy-plus binary")?;
    let mut command = background_command(executable);
    command.arg("--web-login");
    command
        .spawn()
        .context("Failed to launch agy-plus in Google login capture mode")?;
    Ok(())
}

pub async fn capture_google_login(paths: AppPaths) {
    loop {
        tokio::time::sleep(Duration::from_millis(1000)).await;
        let capture_paths = paths.clone();
        let result = tokio::task::spawn_blocking(move || {
            let store = default_system_store();
            // Avoid macOS Keychain read prompt during background polling; rely on file write or Windows CredReadW.
            AccountVault::new(capture_paths).capture_live_account(store.as_ref(), cfg!(windows))
        })
        .await;
        match result {
            Ok(Ok(Some(summary))) => {
                append_log(
                    &paths,
                    "account.google_login.completed",
                    json!({"recordId": summary.record_id}),
                );
                return;
            }
            Ok(Ok(None)) => {}
            Ok(Err(error)) => append_log(
                &paths,
                "account.google_login.capture_retry",
                json!({"message": error.to_string()}),
            ),
            Err(error) => append_log(
                &paths,
                "account.google_login.capture_retry",
                json!({"message": error.to_string()}),
            ),
        }
    }
}

pub async fn wait_until_antigravity_starts() -> DiscoveredApp {
    let mut system = System::new();
    loop {
        refresh_process_discovery(&mut system);
        if let Some(process) = system
            .processes()
            .values()
            .find(|process| is_antigravity_root_process(process))
        {
            let pid = process.pid();
            let start_time = process.start_time();
            let debug_port = debug_port_from_command(process.cmd());
            return DiscoveredApp {
                process: AppProcess::Observed {
                    pid,
                    start_time,
                    system,
                },
                debug_port,
            };
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

pub fn observe_antigravity_on_port(debug_port: u16) -> Option<AppProcess> {
    let mut system = System::new();
    refresh_process_discovery(&mut system);
    let process = system.processes().values().find(|process| {
        is_antigravity_root_process(process)
            && debug_port_from_command(process.cmd()) == Some(debug_port)
    })?;
    Some(AppProcess::Observed {
        pid: process.pid(),
        start_time: process.start_time(),
        system,
    })
}

pub async fn wait_until_antigravity_stops(
    mut root_process: Option<AppProcess>,
    debug_port: u16,
) -> anyhow::Result<AppStop> {
    let mut missing = 0_u8;
    let mut cdp_seen_at = tokio::time::Instant::now();
    let mut next_probe = tokio::time::Instant::now() + CDP_PROBE_INTERVAL;
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if let Some(process) = root_process.as_mut() {
            if process
                .is_running()
                .context("Unable to query Antigravity main process state")?
            {
                missing = 0;
                let now = tokio::time::Instant::now();
                if now >= next_probe {
                    next_probe = now + CDP_PROBE_INTERVAL;
                    if crate::cdp::endpoint_available(debug_port).await {
                        cdp_seen_at = now;
                    } else if now.duration_since(cdp_seen_at) >= CDP_LOST_AFTER {
                        return Ok(AppStop::CdpLost);
                    }
                }
                continue;
            }
        }

        if !crate::cdp::endpoint_available(debug_port).await {
            missing = missing.saturating_add(1);
            if missing >= 5 {
                return Ok(AppStop::Exited);
            }
        } else {
            missing = 0;
        }
    }
}

pub(crate) fn background_command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn debug_port_from_command(command: &[std::ffi::OsString]) -> Option<u16> {
    command.iter().enumerate().find_map(|(index, argument)| {
        let argument = argument.to_str()?;
        let value = if argument == "--remote-debugging-port" {
            command.get(index + 1)?.to_str()?
        } else {
            argument.strip_prefix("--remote-debugging-port=")?
        };
        value.parse::<u16>().ok().filter(|port| *port != 0)
    })
}

fn refresh_process_discovery(system: &mut System) {
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
}

fn refresh_process_identity(system: &mut System, processes: ProcessesToUpdate<'_>) {
    system.refresh_processes_specifics(
        processes,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
}

fn wait_for_pids(system: &mut System, mut pids: Vec<Pid>) -> anyhow::Result<()> {
    for _ in 0..100 {
        if pids.is_empty() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
        refresh_process_identity(system, ProcessesToUpdate::Some(&pids));
        pids.retain(|pid| system.process(*pid).is_some());
    }
    if pids.is_empty() {
        Ok(())
    } else {
        bail!("Timed out waiting for existing Antigravity / agy-plus processes to exit")
    }
}

fn explicit_app_target() -> Option<AntigravityTarget> {
    std::env::var_os("AGY_PLUS_APP_PATH")
        .filter(|value| !value.is_empty())
        .and_then(|value| normalize_app_candidate(Path::new(&value)))
}

fn normalize_app_candidate(path: &Path) -> Option<AntigravityTarget> {
    if path.is_file() {
        #[cfg(target_os = "macos")]
        let install_root = path
            .ancestors()
            .find(|a| a.extension().and_then(|e| e.to_str()) == Some("app"))
            .unwrap_or_else(|| path.parent().unwrap_or(path))
            .to_path_buf();
        #[cfg(not(target_os = "macos"))]
        let install_root = path.parent().unwrap_or(path).to_path_buf();
        return Some(AntigravityTarget {
            executable: path.to_path_buf(),
            install_root,
        });
    }

    #[cfg(target_os = "macos")]
    {
        let exe = path.join("Contents").join("MacOS").join("Antigravity");
        if exe.is_file() {
            return Some(AntigravityTarget {
                executable: exe,
                install_root: path.to_path_buf(),
            });
        }
    }

    #[cfg(windows)]
    {
        for name in ["Antigravity.exe", "antigravity.exe"] {
            let exe = path.join(name);
            if exe.is_file() {
                return Some(AntigravityTarget {
                    executable: exe,
                    install_root: path.to_path_buf(),
                });
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let exe = path.join("antigravity");
        if exe.is_file() {
            return Some(AntigravityTarget {
                executable: exe,
                install_root: path.to_path_buf(),
            });
        }
    }

    None
}

fn is_antigravity_root_executable(name: &OsStr, executable: Option<&Path>) -> bool {
    if let (Some(executable), Some(explicit)) = (executable, explicit_app_target()) {
        if normalized_path(executable) == normalized_path(&explicit.executable) {
            return true;
        }
    }
    let name = name.to_string_lossy().to_ascii_lowercase();
    if !matches!(name.as_str(), "antigravity" | "antigravity.exe") {
        return false;
    }
    let Some(executable) = executable else {
        return false;
    };
    let path = normalized_path(executable);
    #[cfg(target_os = "macos")]
    {
        path == "/applications/antigravity.app/contents/macos/antigravity"
            || path.ends_with("/applications/antigravity.app/contents/macos/antigravity")
    }
    #[cfg(windows)]
    {
        path.ends_with("\\programs\\antigravity\\antigravity.exe")
            || path.ends_with("\\antigravity\\antigravity.exe")
    }
    #[cfg(target_os = "linux")]
    {
        path.ends_with("/antigravity")
    }
}

/// Matches the Antigravity Electron root process as well as helper & language_server children
/// running inside the Antigravity application bundle/install directory.
fn is_antigravity_bundle_process(name: &OsStr, executable: Option<&Path>) -> bool {
    if is_antigravity_root_executable(name, executable) {
        return true;
    }
    let Some(executable) = executable else {
        return false;
    };
    let path = normalized_path(executable);
    if let Some(explicit) = explicit_app_target() {
        let root = normalized_path(&explicit.install_root);
        if path.starts_with(&root) {
            return true;
        }
    }
    #[cfg(target_os = "macos")]
    {
        path.contains("/applications/antigravity.app/contents/")
    }
    #[cfg(windows)]
    {
        path.contains("\\programs\\antigravity\\")
            || path.contains("\\program files\\antigravity\\")
            || path.contains("\\program files (x86)\\antigravity\\")
    }
    #[cfg(target_os = "linux")]
    {
        path.contains("/antigravity/")
    }
}

fn is_antigravity_root_process(process: &sysinfo::Process) -> bool {
    is_antigravity_root_executable(process.name(), process.exe())
        && !is_auxiliary_command(process.cmd())
}

fn is_auxiliary_command(command: &[std::ffi::OsString]) -> bool {
    command.iter().any(|argument| {
        let arg = argument.to_string_lossy().to_ascii_lowercase();
        arg.starts_with("--type=")
    })
}

fn is_agy_plus_launcher(name: &OsStr) -> bool {
    matches!(
        name.to_string_lossy().to_ascii_lowercase().as_str(),
        "agy-plus" | "agy-plus.exe"
    )
}

fn normalized_path(path: &Path) -> String {
    let value = path.to_string_lossy().to_ascii_lowercase();
    #[cfg(windows)]
    return value.replace('/', "\\");
    #[cfg(not(windows))]
    return value.replace('\\', "/");
}

fn sanitize_event(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_port_parsing_handles_equals_separate_and_invalid_values() {
        for (args, expected) in [
            (
                vec!["Antigravity", "--remote-debugging-port=9239"],
                Some(9239),
            ),
            (
                vec!["Antigravity", "--remote-debugging-port", "51234"],
                Some(51234),
            ),
            (vec!["Antigravity"], None),
            (vec!["Antigravity", "--remote-debugging-port=0"], None),
            (vec!["Antigravity", "--remote-debugging-port=99999"], None),
        ] {
            let args: Vec<_> = args.into_iter().map(Into::into).collect();
            assert_eq!(debug_port_from_command(&args), expected);
        }
    }

    #[test]
    fn pending_action_contains_no_tokens() {
        let action = PendingAction::SwitchAccount {
            record_id: "a".repeat(64),
        };
        let encoded = serde_json::to_string(&action).unwrap();
        assert!(!encoded.contains("token"));
        assert!(encoded.contains("switchAccount"));

        let google = serde_json::to_string(&PendingAction::GoogleLogin).unwrap();
        assert_eq!(google, r#"{"kind":"googleLogin"}"#);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_process_matching_includes_bundle_helpers_and_excludes_unrelated() {
        assert!(is_antigravity_root_executable(
            OsStr::new("Antigravity"),
            Some(Path::new(
                "/Applications/Antigravity.app/Contents/MacOS/Antigravity"
            ))
        ));
        assert!(is_antigravity_bundle_process(
            OsStr::new("language_server"),
            Some(Path::new(
                "/Applications/Antigravity.app/Contents/Resources/bin/language_server"
            ))
        ));
        assert!(!is_antigravity_bundle_process(
            OsStr::new("agy"),
            Some(Path::new("/usr/local/bin/agy"))
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_process_matching_includes_installed_antigravity_and_helpers() {
        assert!(is_antigravity_root_executable(
            OsStr::new("Antigravity.exe"),
            Some(Path::new(
                r"C:\Users\Meta\AppData\Local\Programs\Antigravity\Antigravity.exe"
            ))
        ));
        assert!(is_antigravity_bundle_process(
            OsStr::new("language_server.exe"),
            Some(Path::new(
                r"C:\Users\Meta\AppData\Local\Programs\Antigravity\resources\bin\language_server.exe"
            ))
        ));
        assert!(!is_antigravity_bundle_process(
            OsStr::new("Antigravity.exe"),
            Some(Path::new(r"D:\Other\Antigravity.exe"))
        ));
    }

    #[tokio::test]
    async fn select_debug_port_waits_for_release_and_falls_back_when_occupied() {
        let occupied = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let preferred = occupied.local_addr().unwrap().port();
        let selected = select_debug_port(preferred).await.unwrap();
        assert_ne!(selected.port, preferred);
        assert!(selected.preferred_bind_error.is_some());
        assert!(selected.preferred_bind_attempts > 1);

        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            drop(occupied);
        });
        let recovered = select_debug_port(preferred).await.unwrap();
        release.join().unwrap();
        assert_eq!(recovered.port, preferred);
        assert!(recovered.preferred_bind_error.is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn detects_stable_vs_adhoc_codesign_requirement() {
        let adhoc = "Executable=/tmp/x\n# designated => cdhash H\"deadbeef\"\n";
        let req = parse_designated_requirement(adhoc).unwrap();
        assert!(!requirement_uses_stable_identity(&req));

        let signed = "designated => identifier \"com.alexbeatsz.agy-plus\" and certificate leaf = H\"1234\"\n";
        let req = parse_designated_requirement(signed).unwrap();
        assert!(requirement_uses_stable_identity(&req));
    }
}
