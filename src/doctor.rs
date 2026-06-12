use std::{fs, net::TcpListener, path::Path, time::Duration};

use anyhow::Result;
use reqwest::StatusCode;
use serde::Serialize;
use serde_json::{Value, json};

use crate::config::{ALLOWED_EXTENSION_ID, BridgeConfig, EXTENSION_VERSION};

const SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DoctorStatus {
    Ok,
    Degraded,
    Fail,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CheckKind {
    Prerequisite,
    Readiness,
    Advisory,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecoveryAction {
    StartBridge,
    RunInstallEdge,
    RunInstallChrome,
    RunInstallBrowser,
    LoadUnpackedExtension,
    ReloadExtension,
    DisableLegacyExtension,
    StopConflictingProcess,
    UseDifferentPort,
    UpgradeBinary,
    FixTokenFile,
    RepairInstall,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheck {
    pub id: &'static str,
    pub status: CheckStatus,
    pub message: String,
    pub required: bool,
    pub kind: CheckKind,
    pub recovery: Vec<RecoveryAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub schema_version: u8,
    pub status: DoctorStatus,
    pub summary: String,
    pub checks: Vec<DoctorCheck>,
    pub recovery: Vec<RecoveryAction>,
}

pub async fn diagnose(config: &BridgeConfig) -> DoctorReport {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(700))
        .build()
        .expect("valid reqwest client");
    let mut checks = vec![
        binary_version_check(),
        app_dir_check(config),
        app_dir_permissions_check(config),
        token_file_check(config),
        token_permissions_check(config),
        extension_copy_check(config),
        extension_version_check(config),
        pid_file_check(config),
    ];

    let health_probe = probe_health(&client, config).await;
    checks.push(http_port_check(config, &health_probe));
    checks.push(health_endpoint_check(&health_probe));
    checks.push(server_identity_check(&health_probe));
    checks.push(server_version_check(&health_probe));
    checks.push(pid_match_check(config, &health_probe));
    checks.push(ws_port_check(config, &health_probe));
    checks.push(extension_id_check(&health_probe));
    checks.push(extension_origin_check(config, &health_probe));
    checks.push(extension_connection_check(&health_probe));
    checks.push(browser_detection_check());

    build_report(checks)
}

fn build_report(checks: Vec<DoctorCheck>) -> DoctorReport {
    let status = aggregate_status(&checks);
    let recovery = aggregate_recovery(&checks);
    let summary = summary_for(status, &checks);
    DoctorReport {
        schema_version: SCHEMA_VERSION,
        status,
        summary,
        checks,
        recovery,
    }
}

fn aggregate_status(checks: &[DoctorCheck]) -> DoctorStatus {
    if checks
        .iter()
        .any(|c| c.kind == CheckKind::Prerequisite && c.status == CheckStatus::Fail)
    {
        return DoctorStatus::Fail;
    }
    if checks.iter().any(|c| {
        (c.kind == CheckKind::Prerequisite && c.status == CheckStatus::Warn)
            || (c.kind == CheckKind::Readiness
                && matches!(c.status, CheckStatus::Fail | CheckStatus::Warn))
    }) {
        return DoctorStatus::Degraded;
    }
    DoctorStatus::Ok
}

fn aggregate_recovery(checks: &[DoctorCheck]) -> Vec<RecoveryAction> {
    let mut out = Vec::new();
    for action in checks.iter().flat_map(|c| c.recovery.iter().copied()) {
        if !out.contains(&action) {
            out.push(action);
        }
    }
    out
}

fn summary_for(status: DoctorStatus, checks: &[DoctorCheck]) -> String {
    match status {
        DoctorStatus::Ok => "Bridge is ready for browser page work.".to_string(),
        DoctorStatus::Fail => checks
            .iter()
            .find(|c| c.kind == CheckKind::Prerequisite && c.status == CheckStatus::Fail)
            .map(|c| c.message.clone())
            .unwrap_or_else(|| "Core prerequisites are not satisfied.".to_string()),
        DoctorStatus::Degraded => checks
            .iter()
            .find(|c| c.kind == CheckKind::Readiness && c.status != CheckStatus::Ok)
            .map(|c| c.message.clone())
            .or_else(|| {
                checks
                    .iter()
                    .find(|c| c.kind == CheckKind::Prerequisite && c.status == CheckStatus::Warn)
                    .map(|c| c.message.clone())
            })
            .unwrap_or_else(|| "Bridge is installed but not ready yet.".to_string()),
    }
}

fn check(
    id: &'static str,
    kind: CheckKind,
    status: CheckStatus,
    message: impl Into<String>,
    recovery: Vec<RecoveryAction>,
    details: Option<Value>,
) -> DoctorCheck {
    DoctorCheck {
        id,
        status,
        message: message.into(),
        required: kind == CheckKind::Prerequisite,
        kind,
        recovery,
        details,
    }
}

fn binary_version_check() -> DoctorCheck {
    check(
        "binary_version",
        CheckKind::Prerequisite,
        CheckStatus::Ok,
        format!("Binary version is {}.", env!("CARGO_PKG_VERSION")),
        vec![],
        Some(json!({"version": env!("CARGO_PKG_VERSION")})),
    )
}

fn app_dir_check(config: &BridgeConfig) -> DoctorCheck {
    match fs::metadata(&config.app_dir) {
        Ok(meta) if meta.is_dir() => check(
            "app_dir",
            CheckKind::Prerequisite,
            CheckStatus::Ok,
            "App dir exists.",
            vec![],
            Some(json!({"path": config.app_dir})),
        ),
        Ok(_) => check(
            "app_dir",
            CheckKind::Prerequisite,
            CheckStatus::Fail,
            "App dir path exists but is not a directory.",
            vec![RecoveryAction::RepairInstall],
            Some(json!({"path": config.app_dir})),
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => check(
            "app_dir",
            CheckKind::Prerequisite,
            CheckStatus::Fail,
            "App dir is missing.",
            vec![RecoveryAction::RunInstallBrowser],
            Some(json!({"path": config.app_dir})),
        ),
        Err(err) => check(
            "app_dir",
            CheckKind::Prerequisite,
            CheckStatus::Fail,
            format!("App dir is not readable: {err}."),
            vec![RecoveryAction::RepairInstall],
            Some(json!({"path": config.app_dir})),
        ),
    }
}

#[cfg(unix)]
fn app_dir_permissions_check(config: &BridgeConfig) -> DoctorCheck {
    use std::os::unix::fs::PermissionsExt;
    match fs::metadata(&config.app_dir) {
        Ok(meta) if meta.is_dir() => {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 == 0 {
                check(
                    "app_dir_permissions",
                    CheckKind::Advisory,
                    CheckStatus::Ok,
                    "App dir permissions are private.",
                    vec![],
                    Some(json!({"mode": format!("{mode:o}")})),
                )
            } else {
                check(
                    "app_dir_permissions",
                    CheckKind::Advisory,
                    CheckStatus::Warn,
                    "App dir permissions are broader than recommended.",
                    vec![RecoveryAction::RepairInstall],
                    Some(json!({"mode": format!("{mode:o}")})),
                )
            }
        }
        _ => check(
            "app_dir_permissions",
            CheckKind::Advisory,
            CheckStatus::Unknown,
            "App dir permissions could not be checked.",
            vec![],
            None,
        ),
    }
}

#[cfg(not(unix))]
fn app_dir_permissions_check(_config: &BridgeConfig) -> DoctorCheck {
    check(
        "app_dir_permissions",
        CheckKind::Advisory,
        CheckStatus::Unknown,
        "App dir permission check is not available on this platform.",
        vec![],
        None,
    )
}

fn token_file_check(config: &BridgeConfig) -> DoctorCheck {
    match fs::read_to_string(config.token_path()) {
        Ok(token) if !token.trim().is_empty() => check(
            "token_file",
            CheckKind::Readiness,
            CheckStatus::Ok,
            "Token file is present and readable.",
            vec![],
            Some(json!({"path": config.token_path(), "present": true})),
        ),
        Ok(_) => check(
            "token_file",
            CheckKind::Readiness,
            CheckStatus::Fail,
            "Token file is empty.",
            vec![RecoveryAction::FixTokenFile],
            Some(json!({"path": config.token_path(), "present": true})),
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => check(
            "token_file",
            CheckKind::Readiness,
            CheckStatus::Fail,
            "Token file is missing.",
            vec![RecoveryAction::StartBridge, RecoveryAction::FixTokenFile],
            Some(json!({"path": config.token_path(), "present": false})),
        ),
        Err(err) => check(
            "token_file",
            CheckKind::Readiness,
            CheckStatus::Fail,
            format!("Token file is not readable: {err}."),
            vec![RecoveryAction::FixTokenFile],
            Some(json!({"path": config.token_path(), "present": true})),
        ),
    }
}

#[cfg(unix)]
fn token_permissions_check(config: &BridgeConfig) -> DoctorCheck {
    use std::os::unix::fs::PermissionsExt;
    match fs::metadata(config.token_path()) {
        Ok(meta) => {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 == 0 {
                check(
                    "token_permissions",
                    CheckKind::Advisory,
                    CheckStatus::Ok,
                    "Token file permissions are private.",
                    vec![],
                    Some(json!({"mode": format!("{mode:o}")})),
                )
            } else {
                check(
                    "token_permissions",
                    CheckKind::Advisory,
                    CheckStatus::Warn,
                    "Token file permissions are broader than recommended.",
                    vec![RecoveryAction::FixTokenFile],
                    Some(json!({"mode": format!("{mode:o}")})),
                )
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => check(
            "token_permissions",
            CheckKind::Advisory,
            CheckStatus::Unknown,
            "Token file permissions could not be checked because the token is missing.",
            vec![],
            None,
        ),
        Err(err) => check(
            "token_permissions",
            CheckKind::Advisory,
            CheckStatus::Unknown,
            format!("Token file permissions could not be checked: {err}."),
            vec![],
            None,
        ),
    }
}

#[cfg(not(unix))]
fn token_permissions_check(_config: &BridgeConfig) -> DoctorCheck {
    check(
        "token_permissions",
        CheckKind::Advisory,
        CheckStatus::Unknown,
        "Token file permission check is not available on this platform.",
        vec![],
        None,
    )
}

fn extension_copy_check(config: &BridgeConfig) -> DoctorCheck {
    match fs::metadata(config.extension_dir()) {
        Ok(meta) if meta.is_dir() => check(
            "extension_copy",
            CheckKind::Prerequisite,
            CheckStatus::Ok,
            "Installed extension copy is present.",
            vec![],
            Some(json!({"path": config.extension_dir()})),
        ),
        Ok(_) => check(
            "extension_copy",
            CheckKind::Prerequisite,
            CheckStatus::Fail,
            "Installed extension path exists but is not a directory.",
            vec![
                RecoveryAction::RunInstallBrowser,
                RecoveryAction::LoadUnpackedExtension,
            ],
            Some(json!({"path": config.extension_dir()})),
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => check(
            "extension_copy",
            CheckKind::Prerequisite,
            CheckStatus::Fail,
            "Installed extension copy is missing.",
            vec![
                RecoveryAction::RunInstallBrowser,
                RecoveryAction::LoadUnpackedExtension,
            ],
            Some(json!({"path": config.extension_dir()})),
        ),
        Err(err) => check(
            "extension_copy",
            CheckKind::Prerequisite,
            CheckStatus::Fail,
            format!("Installed extension copy is not readable: {err}."),
            vec![
                RecoveryAction::RunInstallBrowser,
                RecoveryAction::LoadUnpackedExtension,
            ],
            Some(json!({"path": config.extension_dir()})),
        ),
    }
}

fn extension_version_check(config: &BridgeConfig) -> DoctorCheck {
    match config.installed_extension_version() {
        Ok(Some(version)) if version == EXTENSION_VERSION => check(
            "extension_version",
            CheckKind::Prerequisite,
            CheckStatus::Ok,
            format!("Installed extension version is {version}."),
            vec![],
            Some(json!({"installed": version, "expected": EXTENSION_VERSION})),
        ),
        Ok(Some(version)) => check(
            "extension_version",
            CheckKind::Prerequisite,
            CheckStatus::Fail,
            format!("Installed extension version is {version}; expected {EXTENSION_VERSION}."),
            vec![
                RecoveryAction::RunInstallBrowser,
                RecoveryAction::LoadUnpackedExtension,
            ],
            Some(json!({"installed": version, "expected": EXTENSION_VERSION})),
        ),
        Ok(None) => check(
            "extension_version",
            CheckKind::Prerequisite,
            CheckStatus::Fail,
            "Installed extension version file is missing.",
            vec![
                RecoveryAction::RunInstallBrowser,
                RecoveryAction::LoadUnpackedExtension,
            ],
            Some(json!({"installed": Value::Null, "expected": EXTENSION_VERSION})),
        ),
        Err(err) => check(
            "extension_version",
            CheckKind::Prerequisite,
            CheckStatus::Fail,
            format!("Installed extension version is not readable: {err}."),
            vec![
                RecoveryAction::RunInstallBrowser,
                RecoveryAction::LoadUnpackedExtension,
            ],
            Some(json!({"installed": Value::Null, "expected": EXTENSION_VERSION})),
        ),
    }
}

fn pid_file_check(config: &BridgeConfig) -> DoctorCheck {
    match fs::read_to_string(config.pid_path()) {
        Ok(pid) => match pid.trim().parse::<u64>() {
            Ok(pid) => check(
                "pid_file",
                CheckKind::Advisory,
                CheckStatus::Ok,
                format!("Pid file contains pid {pid}."),
                vec![],
                Some(json!({"path": config.pid_path(), "present": true, "pid": pid})),
            ),
            Err(err) => check(
                "pid_file",
                CheckKind::Advisory,
                CheckStatus::Warn,
                format!("Pid file is not parseable: {err}."),
                vec![RecoveryAction::RepairInstall],
                Some(json!({"path": config.pid_path(), "present": true})),
            ),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => check(
            "pid_file",
            CheckKind::Advisory,
            CheckStatus::Ok,
            "Pid file is absent.",
            vec![],
            Some(json!({"path": config.pid_path(), "present": false})),
        ),
        Err(err) => check(
            "pid_file",
            CheckKind::Advisory,
            CheckStatus::Warn,
            format!("Pid file is not readable: {err}."),
            vec![RecoveryAction::RepairInstall],
            Some(json!({"path": config.pid_path(), "present": true})),
        ),
    }
}

#[derive(Debug, Clone)]
enum HealthProbe {
    Unreachable,
    NonSuccess { status: u16 },
    InvalidJson { status: u16, error: String },
    NonBridge { status: u16 },
    Bridge(Value),
}

async fn probe_health(client: &reqwest::Client, config: &BridgeConfig) -> HealthProbe {
    let url = format!("http://127.0.0.1:{}/health", config.http_port);
    let Ok(response) = client.get(&url).send().await else {
        return HealthProbe::Unreachable;
    };
    let status = response.status();
    if !status.is_success() {
        return HealthProbe::NonSuccess {
            status: status.as_u16(),
        };
    }
    match response.json::<Value>().await {
        Ok(body) if body.get("server").and_then(Value::as_str) == Some("tmwd-cdp-bridge") => {
            HealthProbe::Bridge(body)
        }
        Ok(_body) => HealthProbe::NonBridge {
            status: StatusCode::OK.as_u16(),
        },
        Err(err) => HealthProbe::InvalidJson {
            status: StatusCode::OK.as_u16(),
            error: err.to_string(),
        },
    }
}

fn http_port_check(config: &BridgeConfig, health: &HealthProbe) -> DoctorCheck {
    match health {
        HealthProbe::Unreachable => match TcpListener::bind(("127.0.0.1", config.http_port)) {
            Ok(listener) => {
                drop(listener);
                check(
                    "http_port",
                    CheckKind::Prerequisite,
                    CheckStatus::Ok,
                    "HTTP port is currently free.",
                    vec![RecoveryAction::StartBridge],
                    Some(json!({"port": config.http_port, "reachable": false})),
                )
            }
            Err(err) => check(
                "http_port",
                CheckKind::Prerequisite,
                CheckStatus::Fail,
                format!(
                    "HTTP port {} is occupied but /health is unreachable: {err}.",
                    config.http_port
                ),
                vec![
                    RecoveryAction::StopConflictingProcess,
                    RecoveryAction::UseDifferentPort,
                ],
                Some(json!({"port": config.http_port, "reachable": false})),
            ),
        },
        HealthProbe::Bridge(_) => check(
            "http_port",
            CheckKind::Prerequisite,
            CheckStatus::Ok,
            "HTTP port is owned by tmwd-cdp-bridge.",
            vec![],
            Some(json!({"port": config.http_port, "reachable": true})),
        ),
        HealthProbe::NonSuccess { status } => check(
            "http_port",
            CheckKind::Prerequisite,
            CheckStatus::Fail,
            format!(
                "HTTP port {} is occupied by a non-bridge service returning {status}.",
                config.http_port
            ),
            vec![
                RecoveryAction::StopConflictingProcess,
                RecoveryAction::UseDifferentPort,
            ],
            Some(json!({"port": config.http_port, "reachable": true, "status": status})),
        ),
        HealthProbe::InvalidJson { status, .. } | HealthProbe::NonBridge { status } => check(
            "http_port",
            CheckKind::Prerequisite,
            CheckStatus::Fail,
            format!(
                "HTTP port {} is occupied by a non-bridge service.",
                config.http_port
            ),
            vec![
                RecoveryAction::StopConflictingProcess,
                RecoveryAction::UseDifferentPort,
            ],
            Some(json!({"port": config.http_port, "reachable": true, "status": status})),
        ),
    }
}

fn health_endpoint_check(health: &HealthProbe) -> DoctorCheck {
    match health {
        HealthProbe::Bridge(_) => check(
            "health_endpoint",
            CheckKind::Readiness,
            CheckStatus::Ok,
            "Health endpoint is reachable.",
            vec![],
            None,
        ),
        HealthProbe::Unreachable => check(
            "health_endpoint",
            CheckKind::Readiness,
            CheckStatus::Fail,
            "Health endpoint is not reachable.",
            vec![RecoveryAction::StartBridge],
            None,
        ),
        HealthProbe::NonSuccess { status } => check(
            "health_endpoint",
            CheckKind::Readiness,
            CheckStatus::Fail,
            format!("Health endpoint returned HTTP {status}."),
            vec![
                RecoveryAction::StopConflictingProcess,
                RecoveryAction::UseDifferentPort,
            ],
            Some(json!({"status": status})),
        ),
        HealthProbe::InvalidJson { status, error } => check(
            "health_endpoint",
            CheckKind::Readiness,
            CheckStatus::Fail,
            format!("Health endpoint returned invalid JSON: {error}."),
            vec![
                RecoveryAction::StopConflictingProcess,
                RecoveryAction::UseDifferentPort,
            ],
            Some(json!({"status": status})),
        ),
        HealthProbe::NonBridge { status } => check(
            "health_endpoint",
            CheckKind::Readiness,
            CheckStatus::Fail,
            "Health endpoint is not tmwd-cdp-bridge.",
            vec![
                RecoveryAction::StopConflictingProcess,
                RecoveryAction::UseDifferentPort,
            ],
            Some(json!({"status": status})),
        ),
    }
}

fn server_identity_check(health: &HealthProbe) -> DoctorCheck {
    match health {
        HealthProbe::Bridge(_) => check(
            "server_identity",
            CheckKind::Prerequisite,
            CheckStatus::Ok,
            "Server identity is tmwd-cdp-bridge.",
            vec![],
            None,
        ),
        HealthProbe::Unreachable => check(
            "server_identity",
            CheckKind::Prerequisite,
            CheckStatus::Unknown,
            "Server identity is unknown because /health is not reachable.",
            vec![RecoveryAction::StartBridge],
            None,
        ),
        _ => check(
            "server_identity",
            CheckKind::Prerequisite,
            CheckStatus::Fail,
            "Server identity is not tmwd-cdp-bridge.",
            vec![
                RecoveryAction::StopConflictingProcess,
                RecoveryAction::UseDifferentPort,
            ],
            None,
        ),
    }
}

fn server_version_check(health: &HealthProbe) -> DoctorCheck {
    let expected = env!("CARGO_PKG_VERSION");
    match health {
        HealthProbe::Bridge(body) => {
            let installed = body.get("version").and_then(Value::as_str);
            if installed == Some(expected) {
                check(
                    "server_version",
                    CheckKind::Prerequisite,
                    CheckStatus::Ok,
                    format!("Running bridge version is {expected}."),
                    vec![],
                    Some(json!({"running": installed, "expected": expected})),
                )
            } else {
                check(
                    "server_version",
                    CheckKind::Prerequisite,
                    CheckStatus::Fail,
                    format!(
                        "Running bridge version is {}; expected {expected}.",
                        installed.unwrap_or("unknown")
                    ),
                    vec![
                        RecoveryAction::UpgradeBinary,
                        RecoveryAction::UseDifferentPort,
                    ],
                    Some(json!({"running": installed, "expected": expected})),
                )
            }
        }
        HealthProbe::Unreachable => check(
            "server_version",
            CheckKind::Prerequisite,
            CheckStatus::Unknown,
            "Running bridge version is unknown because /health is not reachable.",
            vec![RecoveryAction::StartBridge],
            Some(json!({"running": Value::Null, "expected": expected})),
        ),
        _ => check(
            "server_version",
            CheckKind::Prerequisite,
            CheckStatus::Fail,
            "Running bridge version is unavailable because another service owns the port.",
            vec![
                RecoveryAction::StopConflictingProcess,
                RecoveryAction::UseDifferentPort,
            ],
            Some(json!({"running": Value::Null, "expected": expected})),
        ),
    }
}

fn pid_match_check(config: &BridgeConfig, health: &HealthProbe) -> DoctorCheck {
    let pid_file = read_pid(config.pid_path());
    let health_pid = match health {
        HealthProbe::Bridge(body) => body.get("pid").and_then(Value::as_u64),
        _ => None,
    };
    match (pid_file, health_pid) {
        (Ok(Some(file_pid)), Some(running_pid)) if file_pid == running_pid => check(
            "pid_match",
            CheckKind::Advisory,
            CheckStatus::Ok,
            "Pid file matches running bridge pid.",
            vec![],
            Some(json!({"pid_file": file_pid, "health_pid": running_pid})),
        ),
        (Ok(Some(file_pid)), Some(running_pid)) => check(
            "pid_match",
            CheckKind::Advisory,
            CheckStatus::Warn,
            "Pid file does not match running bridge pid.",
            vec![RecoveryAction::RepairInstall],
            Some(json!({"pid_file": file_pid, "health_pid": running_pid})),
        ),
        (Ok(None), Some(running_pid)) => check(
            "pid_match",
            CheckKind::Advisory,
            CheckStatus::Warn,
            "Bridge is running but pid file is missing.",
            vec![RecoveryAction::RepairInstall],
            Some(json!({"pid_file": Value::Null, "health_pid": running_pid})),
        ),
        (_, None) => check(
            "pid_match",
            CheckKind::Advisory,
            CheckStatus::Unknown,
            "Pid match could not be checked because the bridge is not running.",
            vec![],
            None,
        ),
        (Err(err), Some(running_pid)) => check(
            "pid_match",
            CheckKind::Advisory,
            CheckStatus::Warn,
            format!("Pid file could not be read: {err}."),
            vec![RecoveryAction::RepairInstall],
            Some(json!({"health_pid": running_pid})),
        ),
    }
}

fn ws_port_check(config: &BridgeConfig, health: &HealthProbe) -> DoctorCheck {
    if matches!(health, HealthProbe::Bridge(_)) {
        return check(
            "ws_port",
            CheckKind::Advisory,
            CheckStatus::Ok,
            "WebSocket port ownership is implied by compatible HTTP health.",
            vec![],
            Some(json!({"port": config.ws_port})),
        );
    }
    match TcpListener::bind(("127.0.0.1", config.ws_port)) {
        Ok(listener) => {
            drop(listener);
            check(
                "ws_port",
                CheckKind::Advisory,
                CheckStatus::Ok,
                "WebSocket port is currently free.",
                vec![],
                Some(json!({"port": config.ws_port})),
            )
        }
        Err(err) => check(
            "ws_port",
            CheckKind::Advisory,
            CheckStatus::Warn,
            format!("WebSocket port {} appears occupied: {err}.", config.ws_port),
            vec![
                RecoveryAction::StopConflictingProcess,
                RecoveryAction::UseDifferentPort,
            ],
            Some(json!({"port": config.ws_port})),
        ),
    }
}

fn extension_id_check(health: &HealthProbe) -> DoctorCheck {
    match health {
        HealthProbe::Bridge(body) => {
            let id = body.get("extension_id").and_then(Value::as_str);
            if id == Some(ALLOWED_EXTENSION_ID) {
                check(
                    "extension_id",
                    CheckKind::Prerequisite,
                    CheckStatus::Ok,
                    "Bridge reports the expected extension id.",
                    vec![],
                    Some(json!({"extension_id": id})),
                )
            } else {
                check(
                    "extension_id",
                    CheckKind::Prerequisite,
                    CheckStatus::Fail,
                    format!(
                        "Bridge reports unexpected extension id {}.",
                        id.unwrap_or("unknown")
                    ),
                    vec![
                        RecoveryAction::UpgradeBinary,
                        RecoveryAction::UseDifferentPort,
                    ],
                    Some(json!({"extension_id": id, "expected": ALLOWED_EXTENSION_ID})),
                )
            }
        }
        HealthProbe::Unreachable => check(
            "extension_id",
            CheckKind::Prerequisite,
            CheckStatus::Unknown,
            "Extension id is unknown because /health is not reachable.",
            vec![RecoveryAction::StartBridge],
            Some(json!({"expected": ALLOWED_EXTENSION_ID})),
        ),
        _ => check(
            "extension_id",
            CheckKind::Prerequisite,
            CheckStatus::Unknown,
            "Extension id is unknown because the running service is not a compatible bridge.",
            vec![],
            Some(json!({"expected": ALLOWED_EXTENSION_ID})),
        ),
    }
}

fn extension_origin_check(config: &BridgeConfig, health: &HealthProbe) -> DoctorCheck {
    match health {
        HealthProbe::Bridge(body) => {
            let origin = body.get("allowed_extension_origin").and_then(Value::as_str);
            if origin == Some(config.allowed_extension_origin.as_str()) {
                check(
                    "extension_origin",
                    CheckKind::Prerequisite,
                    CheckStatus::Ok,
                    "Bridge reports the expected extension origin.",
                    vec![],
                    Some(json!({"allowed_extension_origin": origin})),
                )
            } else {
                check(
                    "extension_origin",
                    CheckKind::Prerequisite,
                    CheckStatus::Fail,
                    format!(
                        "Bridge reports unexpected extension origin {}.",
                        origin.unwrap_or("unknown")
                    ),
                    vec![
                        RecoveryAction::UpgradeBinary,
                        RecoveryAction::UseDifferentPort,
                    ],
                    Some(json!({
                        "allowed_extension_origin": origin,
                        "configured": config.allowed_extension_origin,
                    })),
                )
            }
        }
        HealthProbe::Unreachable => check(
            "extension_origin",
            CheckKind::Prerequisite,
            CheckStatus::Unknown,
            "Extension origin is unknown because /health is not reachable.",
            vec![RecoveryAction::StartBridge],
            Some(json!({"configured": config.allowed_extension_origin})),
        ),
        _ => check(
            "extension_origin",
            CheckKind::Prerequisite,
            CheckStatus::Unknown,
            "Extension origin is unknown because the running service is not a compatible bridge.",
            vec![],
            Some(json!({"configured": config.allowed_extension_origin})),
        ),
    }
}

fn extension_connection_check(health: &HealthProbe) -> DoctorCheck {
    match health {
        HealthProbe::Bridge(body) => match body.get("extension_connected").and_then(Value::as_bool)
        {
            Some(true) => check(
                "extension_connection",
                CheckKind::Readiness,
                CheckStatus::Ok,
                "Extension WebSocket is connected.",
                vec![],
                None,
            ),
            _ => check(
                "extension_connection",
                CheckKind::Readiness,
                CheckStatus::Fail,
                "No extension WebSocket is connected.",
                vec![
                    RecoveryAction::ReloadExtension,
                    RecoveryAction::LoadUnpackedExtension,
                    RecoveryAction::DisableLegacyExtension,
                ],
                None,
            ),
        },
        HealthProbe::Unreachable => check(
            "extension_connection",
            CheckKind::Readiness,
            CheckStatus::Unknown,
            "Extension connection is unknown because the bridge is not running.",
            vec![RecoveryAction::StartBridge],
            None,
        ),
        _ => check(
            "extension_connection",
            CheckKind::Readiness,
            CheckStatus::Unknown,
            "Extension connection is unknown because the running service is not a compatible bridge.",
            vec![],
            None,
        ),
    }
}

fn browser_detection_check() -> DoctorCheck {
    let browsers = detect_browsers();
    if browsers.is_empty() {
        check(
            "browser_detection",
            CheckKind::Advisory,
            CheckStatus::Unknown,
            "Chrome or Edge was not detected from common local paths.",
            vec![],
            Some(json!({"detected": browsers})),
        )
    } else {
        check(
            "browser_detection",
            CheckKind::Advisory,
            CheckStatus::Ok,
            format!("Detected browser candidates: {}.", browsers.join(", ")),
            vec![],
            Some(json!({"detected": browsers})),
        )
    }
}

fn detect_browsers() -> Vec<String> {
    let mut out = Vec::new();
    #[cfg(target_os = "macos")]
    {
        if Path::new("/Applications/Microsoft Edge.app").exists() {
            out.push("edge".to_string());
        }
        if Path::new("/Applications/Google Chrome.app").exists() {
            out.push("chrome".to_string());
        }
    }
    #[cfg(target_os = "linux")]
    {
        for name in [
            "microsoft-edge",
            "google-chrome",
            "chromium",
            "chromium-browser",
        ] {
            if command_on_path(name) {
                out.push(name.to_string());
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        for path in [
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ] {
            if Path::new(path).exists() {
                out.push(path.to_string());
            }
        }
    }
    out
}

#[cfg(target_os = "linux")]
fn command_on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            let path = dir.join(name);
            path.is_file()
        })
    })
}

fn read_pid(path: impl AsRef<Path>) -> Result<Option<u64>> {
    match fs::read_to_string(path.as_ref()) {
        Ok(pid) => Ok(Some(pid.trim().parse::<u64>()?)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_actions_are_deduplicated_in_order() {
        let checks = vec![
            check(
                "a",
                CheckKind::Readiness,
                CheckStatus::Fail,
                "a",
                vec![RecoveryAction::StartBridge, RecoveryAction::FixTokenFile],
                None,
            ),
            check(
                "b",
                CheckKind::Readiness,
                CheckStatus::Fail,
                "b",
                vec![RecoveryAction::StartBridge, RecoveryAction::ReloadExtension],
                None,
            ),
        ];
        assert_eq!(
            aggregate_recovery(&checks),
            vec![
                RecoveryAction::StartBridge,
                RecoveryAction::FixTokenFile,
                RecoveryAction::ReloadExtension
            ]
        );
    }

    #[test]
    fn prerequisite_fail_beats_readiness_fail() {
        let checks = vec![
            check(
                "extension_copy",
                CheckKind::Prerequisite,
                CheckStatus::Fail,
                "missing",
                vec![],
                None,
            ),
            check(
                "extension_connection",
                CheckKind::Readiness,
                CheckStatus::Fail,
                "disconnected",
                vec![],
                None,
            ),
        ];
        assert_eq!(aggregate_status(&checks), DoctorStatus::Fail);
    }

    #[test]
    fn readiness_fail_is_degraded() {
        let checks = vec![check(
            "extension_connection",
            CheckKind::Readiness,
            CheckStatus::Fail,
            "disconnected",
            vec![],
            None,
        )];
        assert_eq!(aggregate_status(&checks), DoctorStatus::Degraded);
    }

    #[test]
    fn prerequisite_unknown_does_not_fail() {
        let checks = vec![check(
            "server_identity",
            CheckKind::Prerequisite,
            CheckStatus::Unknown,
            "unknown",
            vec![],
            None,
        )];
        assert_eq!(aggregate_status(&checks), DoctorStatus::Ok);
    }
}
