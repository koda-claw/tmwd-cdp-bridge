use std::fs;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use reqwest::StatusCode;
use serde_json::{Value, json};

use tmwd_cdp_bridge::{
    auth,
    config::{BridgeConfig, EXTENSION_VERSION},
    install, server,
};

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Start,
    Stop,
    Install { browser: Browser },
    Repair { browser: Option<Browser> },
    Upgrade,
    Status,
    Version,
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
        Command::Stop => stop(config).await,
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
        Command::Upgrade => {
            let instructions = install::install_extension(&config, "edge")?;
            println!("{instructions}");
            println!("Extension version updated to {EXTENSION_VERSION}");
            Ok(())
        }
        Command::Status => status(config).await,
        Command::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
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

async fn status(config: BridgeConfig) -> Result<()> {
    let client = reqwest::Client::new();
    let server = health_status(&client, &config).await;
    let body = json!({
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
    });
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
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

async fn stop(config: BridgeConfig) -> Result<()> {
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
        return Ok(());
    }
    bail!("shutdown failed: {}", response.status())
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
