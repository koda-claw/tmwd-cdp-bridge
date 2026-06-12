use std::fs;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::time::{Duration, sleep};

use tmwd_cdp_bridge::{
    auth,
    config::{BridgeConfig, EXTENSION_VERSION},
    install, self_update, server,
};

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Local Chrome/Edge CDP bridge for agents",
    long_about = "Run and inspect a localhost-only bridge for the bundled TMWD CDP Bridge browser extension.\n\nTypical flow:\n  tmwd-cdp-bridge install edge\n  tmwd-cdp-bridge start\n  tmwd-cdp-bridge status --json"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Run the local HTTP and WebSocket bridge")]
    Start,
    #[command(about = "Stop a verified running bridge")]
    Stop {
        #[arg(long, help = "Emit machine-readable JSON")]
        json: bool,
    },
    #[command(about = "Copy the bundled extension into the platform app data directory")]
    Install { browser: Browser },
    #[command(about = "Print extension loading instructions for browser recovery")]
    Repair { browser: Option<Browser> },
    #[command(about = "Upgrade the tmwd-cdp-bridge CLI binary from GitHub Releases")]
    Upgrade {
        #[arg(long, help = "Release tag to install, for example v0.1.2")]
        version: Option<String>,
        #[arg(long, help = "GitHub repo in owner/name form")]
        repo: Option<String>,
        #[arg(long, help = "Emit machine-readable JSON")]
        json: bool,
    },
    #[command(about = "Show bridge, extension, token, and port status")]
    Status {
        #[arg(long, help = "Emit machine-readable JSON")]
        json: bool,
    },
    #[command(about = "Print the CLI version")]
    Version {
        #[arg(long, help = "Emit machine-readable JSON")]
        json: bool,
    },
}

#[derive(Debug, Clone, ValueEnum)]
enum Browser {
    Edge,
    Chrome,
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    let config = BridgeConfig::from_env()?;
    match args.command {
        Command::Start => server::run_server(config).await,
        Command::Stop { json } => {
            let outcome = stop(config).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&outcome.to_json())?);
            } else {
                println!(
                    "Stopped tmwd-cdp-bridge (pid {}, HTTP port {}).",
                    outcome.pid, outcome.http_port
                );
            }
            Ok(())
        }
        Command::Install { browser } => {
            let instructions = install::install_extension(&config, browser.as_str())?;
            println!("{instructions}");
            Ok(())
        }
        Command::Repair { browser } => {
            let browser = browser.unwrap_or(Browser::Edge);
            println!(
                "{}",
                install::install_instructions(browser.as_str(), &config.extension_dir())
            );
            Ok(())
        }
        Command::Upgrade {
            version,
            repo,
            json,
        } => {
            let outcome =
                self_update::upgrade_current_binary(version.as_deref(), repo.as_deref()).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&outcome)?);
            } else {
                println!(
                    "Upgraded tmwd-cdp-bridge to {} at {}.",
                    outcome.version,
                    outcome.destination.display()
                );
                println!("Source: {}", outcome.source);
                if outcome.pending_restart {
                    println!(
                        "Replacement is scheduled after this process exits; rerun tmwd-cdp-bridge to use the new binary."
                    );
                }
            }
            Ok(())
        }
        Command::Status { json } => status(config, json).await,
        Command::Version { json } => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({"version": env!("CARGO_PKG_VERSION")}))?
                );
            } else {
                println!("{}", env!("CARGO_PKG_VERSION"));
            }
            Ok(())
        }
    }
}

impl Browser {
    fn as_str(&self) -> &'static str {
        match self {
            Browser::Edge => "edge",
            Browser::Chrome => "chrome",
        }
    }
}

async fn status(config: BridgeConfig, json_output: bool) -> Result<()> {
    let body = status_body(config).await;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        print_human_status(&body);
    }
    Ok(())
}

async fn status_body(config: BridgeConfig) -> Value {
    let client = reqwest::Client::new();
    let server = health_status(&client, &config).await;
    json!({
        "server": server,
        "pid_file": pid_file_status(&config),
        "ports": {
            "ws": config.ws_port,
            "http": config.http_port,
        },
        "app_dir": config.app_dir,
        "extension_dir": config.extension_dir(),
        "extension_version": extension_version_status(&config),
        "expected_extension_version": EXTENSION_VERSION,
        "token": token_status(&config),
    })
}

fn print_human_status(body: &Value) {
    let server = &body["server"];
    let ports = &body["ports"];
    let pid_file = &body["pid_file"];
    let extension = &body["extension_version"];

    let http_port = ports["http"].as_u64().unwrap_or(18766);
    let ws_port = ports["ws"].as_u64().unwrap_or(18765);
    let running = server["owned_by_tmwd"].as_bool() == Some(true);
    let server_line = if running {
        match server["pid"].as_u64() {
            Some(pid) => format!("running (pid {pid})"),
            None => "running".to_string(),
        }
    } else if let Some(status) = server["status"].as_u64() {
        format!("not running; HTTP port answered with status {status}")
    } else {
        "not running".to_string()
    };

    let extension_line = match (
        extension["installed"].as_str(),
        extension["ok"].as_bool().unwrap_or(false),
    ) {
        (Some(installed), true) => format!("installed {installed} (ok)"),
        (Some(installed), false) => format!(
            "installed {installed} (expected {})",
            body["expected_extension_version"]
                .as_str()
                .unwrap_or("unknown")
        ),
        (None, _) => "not installed".to_string(),
    };
    let connected_line = if running {
        match server["extension_connected"].as_bool() {
            Some(true) => "connected".to_string(),
            Some(false) => "not connected".to_string(),
            None => "unknown".to_string(),
        }
    } else {
        "bridge not running".to_string()
    };
    let pid_line = match (pid_file["present"].as_bool(), pid_file["pid"].as_u64()) {
        (Some(true), Some(pid)) => format!("present ({pid})"),
        (Some(true), None) => "present (unreadable)".to_string(),
        _ => "absent".to_string(),
    };

    println!("tmwd-cdp-bridge {}", env!("CARGO_PKG_VERSION"));
    println!("Server: {server_line}");
    println!("HTTP:   http://127.0.0.1:{http_port}");
    println!("WS:     ws://127.0.0.1:{ws_port}/");
    println!("Extension files: {extension_line}");
    println!("Extension link:  {connected_line}");
    println!("Token:  {}", body["token"].as_str().unwrap_or("missing"));
    println!("Pid file: {pid_line}");
    println!("App dir: {}", body["app_dir"].as_str().unwrap_or("unknown"));
    println!(
        "Extension dir: {}",
        body["extension_dir"].as_str().unwrap_or("unknown")
    );
    println!();
    println!("Machine-readable: tmwd-cdp-bridge status --json");
    println!("Health endpoint:  curl -s http://127.0.0.1:{http_port}/health");

    if !extension["ok"].as_bool().unwrap_or(false) {
        println!("Next: tmwd-cdp-bridge install edge");
    } else if !running {
        println!("Next: tmwd-cdp-bridge start");
    } else if server["extension_connected"].as_bool() != Some(true) {
        println!("Next: reload the unpacked extension in chrome://extensions or edge://extensions");
    } else {
        println!("Next: use authenticated POST /v1/rpc");
    }
}

async fn health_status(client: &reqwest::Client, config: &BridgeConfig) -> Value {
    let url = format!("http://127.0.0.1:{}/health", config.http_port);
    match client.get(&url).send().await {
        Ok(response) if response.status().is_success() => match response.json::<Value>().await {
            Ok(mut body)
                if body.get("server").and_then(Value::as_str) == Some("tmwd-cdp-bridge") =>
            {
                if let Some(obj) = body.as_object_mut() {
                    obj.insert("running".to_string(), Value::Bool(true));
                    obj.insert("owned_by_tmwd".to_string(), Value::Bool(true));
                }
                body
            }
            Ok(body) => json!({
                "running": false,
                "owned_by_tmwd": false,
                "status": 200,
                "body": body,
            }),
            Err(err) => json!({
                "running": false,
                "owned_by_tmwd": false,
                "status": 200,
                "error": err.to_string(),
            }),
        },
        Ok(response) => json!({
            "running": false,
            "owned_by_tmwd": false,
            "status": response.status().as_u16(),
        }),
        Err(_) => json!({
            "running": false,
            "owned_by_tmwd": false,
        }),
    }
}

fn pid_file_status(config: &BridgeConfig) -> Value {
    match read_pid_file(config) {
        Ok(Some(pid)) => json!({"pid": pid, "present": true}),
        Ok(None) => json!({"pid": null, "present": false}),
        Err(err) => json!({"pid": null, "present": true, "error": err.to_string()}),
    }
}

fn token_status(config: &BridgeConfig) -> String {
    fs::read_to_string(config.token_path())
        .map(|token| auth::token_prefix(token.trim()))
        .unwrap_or_else(|_| "missing".to_string())
}

fn extension_version_status(config: &BridgeConfig) -> Value {
    match config.installed_extension_version() {
        Ok(Some(version)) if version == EXTENSION_VERSION => {
            json!({"installed": version, "expected": EXTENSION_VERSION, "ok": true})
        }
        Ok(Some(version)) => {
            json!({"installed": version, "expected": EXTENSION_VERSION, "ok": false})
        }
        Ok(None) => json!({"installed": null, "expected": EXTENSION_VERSION, "ok": false}),
        Err(err) => {
            json!({"installed": null, "expected": EXTENSION_VERSION, "ok": false, "error": err.to_string()})
        }
    }
}

struct StopOutcome {
    pid: u64,
    http_port: u16,
}

impl StopOutcome {
    fn to_json(&self) -> Value {
        json!({
            "stopped": true,
            "pid": self.pid,
            "http_port": self.http_port,
        })
    }
}

async fn stop(config: BridgeConfig) -> Result<StopOutcome> {
    let client = reqwest::Client::new();
    let health = health_status(&client, &config).await;
    if health.get("owned_by_tmwd").and_then(Value::as_bool) != Some(true) {
        bail!(
            "refusing to stop: no tmwd-cdp-bridge server verified on HTTP port {}",
            config.http_port
        );
    }
    let health_pid = health
        .get("pid")
        .and_then(Value::as_u64)
        .context("verified tmwd-cdp-bridge health response did not include pid")?;
    let pid_file = read_pid_file(&config)?.context("pid file missing; refusing to stop")?;
    if health_pid != pid_file {
        bail!(
            "refusing to stop: pid file {pid_file} does not match running bridge pid {health_pid}"
        );
    }
    let token = fs::read_to_string(config.token_path())
        .context("token missing; server may not be running")?
        .trim()
        .to_string();
    let url = format!("http://127.0.0.1:{}/v1/rpc", config.http_port);
    let response = client
        .post(url)
        .bearer_auth(token)
        .json(&json!({"cmd":"shutdown"}))
        .send()
        .await?;
    if response.status() == StatusCode::OK {
        wait_until_stopped(&client, &config).await?;
        remove_pid_file_if_matches(&config, health_pid)?;
        return Ok(StopOutcome {
            pid: health_pid,
            http_port: config.http_port,
        });
    }
    bail!("shutdown failed: {}", response.status())
}

async fn wait_until_stopped(client: &reqwest::Client, config: &BridgeConfig) -> Result<()> {
    for _ in 0..40 {
        let health = health_status(client, config).await;
        if health.get("owned_by_tmwd").and_then(Value::as_bool) != Some(true) {
            return Ok(());
        }
        sleep(Duration::from_millis(50)).await;
    }
    bail!(
        "shutdown accepted but tmwd-cdp-bridge is still healthy on HTTP port {}",
        config.http_port
    )
}

fn remove_pid_file_if_matches(config: &BridgeConfig, expected_pid: u64) -> Result<()> {
    match read_pid_file(config)? {
        Some(pid) if pid == expected_pid => fs::remove_file(config.pid_path())
            .with_context(|| format!("remove pid {}", config.pid_path().display())),
        Some(pid) => {
            bail!("refusing to remove pid file: expected stopped pid {expected_pid}, found {pid}")
        }
        None => Ok(()),
    }
}

fn read_pid_file(config: &BridgeConfig) -> Result<Option<u64>> {
    match fs::read_to_string(config.pid_path()) {
        Ok(pid) => {
            let pid = pid
                .trim()
                .parse::<u64>()
                .with_context(|| format!("parse pid {}", config.pid_path().display()))?;
            Ok(Some(pid))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("read pid {}", config.pid_path().display())),
    }
}
