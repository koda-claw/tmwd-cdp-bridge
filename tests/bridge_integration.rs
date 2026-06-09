use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tempfile::TempDir;
use tmwd_cdp_bridge::config::{ALLOWED_EXTENSION_ORIGIN, EXTENSION_VERSION};
use tokio::net::TcpStream as TokioTcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};

struct BridgeProcess {
    child: Child,
    app_dir: TempDir,
    ws_port: u16,
    http_port: u16,
}

impl BridgeProcess {
    async fn start() -> Self {
        let app_dir = tempfile::tempdir().expect("temp app dir");
        fs::write(app_dir.path().join("version"), EXTENSION_VERSION).expect("version file");
        let ws_port = free_port();
        let http_port = free_port();
        let child = Command::new(env!("CARGO_BIN_EXE_tmwd-cdp-bridge"))
            .arg("start")
            .env("CDP_BRIDGE_APP_DIR", app_dir.path())
            .env("CDP_BRIDGE_WS_PORT", ws_port.to_string())
            .env("CDP_BRIDGE_HTTP_PORT", http_port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn bridge");
        let bridge = Self {
            child,
            app_dir,
            ws_port,
            http_port,
        };
        bridge.wait_for_health().await;
        bridge
    }

    async fn wait_for_health(&self) {
        let client = reqwest::Client::new();
        let url = self.url("/health");
        for _ in 0..80 {
            if client
                .get(&url)
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("bridge did not become healthy");
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.http_port)
    }

    fn ws_url(&self) -> String {
        format!("ws://127.0.0.1:{}/", self.ws_port)
    }

    fn token(&self) -> String {
        fs::read_to_string(self.app_dir.path().join("token"))
            .expect("token")
            .trim()
            .to_string()
    }
}

impl Drop for BridgeProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind port 0")
        .local_addr()
        .expect("local addr")
        .port()
}

async fn connect_extension(
    bridge: &BridgeProcess,
) -> WebSocketStream<MaybeTlsStream<TokioTcpStream>> {
    let mut req = bridge.ws_url().into_client_request().unwrap();
    req.headers_mut()
        .insert("Origin", ALLOWED_EXTENSION_ORIGIN.parse().unwrap());
    let (mut ws, _) = connect_async(req).await.expect("ws connect");

    let msg: Value =
        serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(msg["type"], "auth_required");

    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type":"auth","token":bridge.token()}).to_string(),
    ))
    .await
    .unwrap();
    let msg: Value =
        serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(msg["type"], "auth_ok");

    ws
}

async fn send_tabs(ws: &mut WebSocketStream<MaybeTlsStream<TokioTcpStream>>, tabs: Value) {
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type":"ext_ready","tabs":tabs}).to_string(),
    ))
    .await
    .unwrap();
}

async fn next_ws_json(ws: &mut WebSocketStream<MaybeTlsStream<TokioTcpStream>>) -> Value {
    serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap()
}

async fn rpc(client: &reqwest::Client, bridge: &BridgeProcess, payload: Value) -> Value {
    client
        .post(bridge.url("/v1/rpc"))
        .bearer_auth(bridge.token())
        .json(&payload)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()
}

async fn rpc_status_and_body(
    client: &reqwest::Client,
    bridge: &BridgeProcess,
    payload: Value,
) -> (reqwest::StatusCode, Value) {
    let response = client
        .post(bridge.url("/v1/rpc"))
        .bearer_auth(bridge.token())
        .json(&payload)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.json::<Value>().await.unwrap();
    (status, body)
}

fn status_json(app_dir: &std::path::Path, ws_port: u16, http_port: u16) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_tmwd-cdp-bridge"))
        .arg("status")
        .arg("--json")
        .env("CDP_BRIDGE_APP_DIR", app_dir)
        .env("CDP_BRIDGE_WS_PORT", ws_port.to_string())
        .env("CDP_BRIDGE_HTTP_PORT", http_port.to_string())
        .output()
        .expect("run status");
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
}

struct TinyHttpServer {
    port: u16,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TinyHttpServer {
    fn start(status: &'static str, body: &'static str) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind tiny http");
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let handle = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => handle_tiny_http(&mut stream, status, body),
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            port,
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for TinyHttpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn handle_tiny_http(stream: &mut TcpStream, status: &str, body: &str) {
    let mut buf = [0_u8; 1024];
    let _ = stream.read(&mut buf);
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

async fn wait_until_unhealthy(url: &str) {
    let client = reqwest::Client::new();
    for _ in 0..40 {
        if client.get(url).send().await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{url} stayed healthy");
}

#[tokio::test]
async fn http_health_and_rpc_auth_work() {
    let bridge = BridgeProcess::start().await;
    let client = reqwest::Client::new();

    let health: Value = client
        .get(bridge.url("/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["server"], "tmwd-cdp-bridge");
    assert_eq!(health["extension_id"], "eghifjkffmcmffejmaaeicejpfopplem");
    assert_eq!(
        health["allowed_extension_origin"],
        "chrome-extension://eghifjkffmcmffejmaaeicejpfopplem"
    );
    assert_eq!(health["extension_connected"], false);
    assert_eq!(health["extension_connected_at_unix_ms"], Value::Null);
    assert_eq!(health["extension_last_seen_at_unix_ms"], Value::Null);
    assert_eq!(health["extension_last_seen_age_ms"], Value::Null);

    let unauth = client
        .post(bridge.url("/v1/rpc"))
        .json(&json!({"cmd":"get_all_sessions"}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    let authed: Value = client
        .post(bridge.url("/v1/rpc"))
        .bearer_auth(bridge.token())
        .json(&json!({"cmd":"get_all_sessions","request_id":"sessions"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(authed["r"]["request_id"], "sessions");
    assert_eq!(authed["r"]["data"], json!([]));

    let removed = client.post(bridge.url("/link")).send().await.unwrap();
    assert_eq!(removed.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rpc_error_contract_covers_auth_bad_request_no_extension_and_no_session() {
    let bridge = BridgeProcess::start().await;
    let client = reqwest::Client::new();

    let unauth: Value = client
        .post(bridge.url("/v1/rpc"))
        .json(&json!({"cmd":"get_all_sessions","request_id":"unauth"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(unauth["r"]["request_id"], "unauth");
    assert_eq!(unauth["r"]["error"]["code"], "UNAUTHORIZED");

    let malformed = client
        .post(bridge.url("/v1/rpc"))
        .bearer_auth(bridge.token())
        .header("Content-Type", "application/json")
        .body("{")
        .send()
        .await
        .unwrap();
    assert_eq!(malformed.status(), reqwest::StatusCode::BAD_REQUEST);
    let malformed: Value = malformed.json().await.unwrap();
    assert_eq!(malformed["r"]["error"]["code"], "BAD_REQUEST");

    let (status, unknown) = rpc_status_and_body(
        &client,
        &bridge,
        json!({"cmd":"nope","request_id":"bad-cmd"}),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(unknown["r"]["request_id"], "bad-cmd");
    assert_eq!(unknown["r"]["error"]["code"], "BAD_REQUEST");

    let no_extension = rpc(
        &client,
        &bridge,
        json!({"cmd":"execute_js","request_id":"no-ext","code":"document.title"}),
    )
    .await;
    assert_eq!(no_extension["r"]["request_id"], "no-ext");
    assert_eq!(no_extension["r"]["error"]["code"], "NO_EXTENSION");

    let mut ws = connect_extension(&bridge).await;
    send_tabs(
        &mut ws,
        json!([{"id":1,"url":"https://example.com","title":"Example","active":true,"window_id":1}]),
    )
    .await;

    let bad_session = rpc(
        &client,
        &bridge,
        json!({"cmd":"execute_js","request_id":"bad-session","sessionId":"abc","code":"document.title"}),
    )
    .await;
    assert_eq!(bad_session["r"]["error"]["code"], "NO_SESSION");

    let missing_code = rpc(
        &client,
        &bridge,
        json!({"cmd":"execute_js","request_id":"missing-code"}),
    )
    .await;
    assert_eq!(missing_code["r"]["error"]["code"], "BAD_REQUEST");

    let bad_fallback = rpc(
        &client,
        &bridge,
        json!({"cmd":"execute_js","request_id":"bad-fallback","fallback":"auto","code":"document.title"}),
    )
    .await;
    assert_eq!(bad_fallback["r"]["error"]["code"], "BAD_REQUEST");
}

#[tokio::test]
async fn start_rejects_missing_or_mismatched_extension_version() {
    let missing_dir = tempfile::tempdir().expect("temp app dir");
    let missing = Command::new(env!("CARGO_BIN_EXE_tmwd-cdp-bridge"))
        .arg("start")
        .env("CDP_BRIDGE_APP_DIR", missing_dir.path())
        .env("CDP_BRIDGE_WS_PORT", free_port().to_string())
        .env("CDP_BRIDGE_HTTP_PORT", free_port().to_string())
        .output()
        .expect("run start missing version");
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.contains("version file missing"));
    assert!(stderr.contains("tmwd-cdp-bridge install"));

    let mismatch_dir = tempfile::tempdir().expect("temp app dir");
    fs::write(mismatch_dir.path().join("version"), "1.0").unwrap();
    let mismatch = Command::new(env!("CARGO_BIN_EXE_tmwd-cdp-bridge"))
        .arg("start")
        .env("CDP_BRIDGE_APP_DIR", mismatch_dir.path())
        .env("CDP_BRIDGE_WS_PORT", free_port().to_string())
        .env("CDP_BRIDGE_HTTP_PORT", free_port().to_string())
        .output()
        .expect("run start mismatched version");
    assert!(!mismatch.status.success());
    let stderr = String::from_utf8_lossy(&mismatch.stderr);
    assert!(stderr.contains("extension version mismatch"));
    assert!(stderr.contains("tmwd-cdp-bridge upgrade"));
}

#[test]
fn status_outputs_structured_runtime_state() {
    let app_dir = tempfile::tempdir().expect("temp app dir");
    fs::write(app_dir.path().join("version"), EXTENSION_VERSION).unwrap();
    fs::write(
        app_dir.path().join("token"),
        "abcdef0123456789abcdef0123456789",
    )
    .unwrap();
    let ws_port = free_port();
    let http_port = free_port();
    let output = Command::new(env!("CARGO_BIN_EXE_tmwd-cdp-bridge"))
        .arg("status")
        .arg("--json")
        .env("CDP_BRIDGE_APP_DIR", app_dir.path())
        .env("CDP_BRIDGE_WS_PORT", ws_port.to_string())
        .env("CDP_BRIDGE_HTTP_PORT", http_port.to_string())
        .output()
        .expect("run status");
    assert!(output.status.success());
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["server"]["running"], false);
    assert_eq!(body["server"]["owned_by_tmwd"], false);
    assert_eq!(body["ports"]["ws"], ws_port);
    assert_eq!(body["ports"]["http"], http_port);
    assert_eq!(body["app_dir"], app_dir.path().to_string_lossy().as_ref());
    assert_eq!(body["extension_version"]["installed"], EXTENSION_VERSION);
    assert_eq!(body["extension_version"]["ok"], true);
    assert_eq!(body["token"], "abcdef01...");
    assert_eq!(body["pid_file"]["present"], false);
}

#[test]
fn status_default_outputs_human_summary() {
    let app_dir = tempfile::tempdir().expect("temp app dir");
    fs::write(app_dir.path().join("version"), EXTENSION_VERSION).unwrap();
    let ws_port = free_port();
    let http_port = free_port();
    let output = Command::new(env!("CARGO_BIN_EXE_tmwd-cdp-bridge"))
        .arg("status")
        .env("CDP_BRIDGE_APP_DIR", app_dir.path())
        .env("CDP_BRIDGE_WS_PORT", ws_port.to_string())
        .env("CDP_BRIDGE_HTTP_PORT", http_port.to_string())
        .output()
        .expect("run status");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tmwd-cdp-bridge"));
    assert!(stdout.contains("Server: not running"));
    assert!(stdout.contains("Machine-readable: tmwd-cdp-bridge status --json"));
    assert!(serde_json::from_slice::<Value>(&output.stdout).is_err());
}

#[tokio::test]
async fn status_reflects_running_server_and_extension_connection() {
    let bridge = BridgeProcess::start().await;

    let body = status_json(bridge.app_dir.path(), bridge.ws_port, bridge.http_port);
    assert_eq!(body["server"]["server"], "tmwd-cdp-bridge");
    assert_eq!(body["server"]["owned_by_tmwd"], true);
    assert_eq!(body["server"]["extension_connected"], false);
    assert_eq!(body["extension_version"]["ok"], true);
    assert_eq!(body["pid_file"]["present"], true);
    assert_eq!(body["pid_file"]["pid"], body["server"]["pid"]);

    let mut ws = connect_extension(&bridge).await;
    send_tabs(&mut ws, json!([])).await;

    let body = status_json(bridge.app_dir.path(), bridge.ws_port, bridge.http_port);
    assert_eq!(body["server"]["server"], "tmwd-cdp-bridge");
    assert_eq!(body["server"]["extension_connected"], true);
    assert_eq!(
        body["server"]["extension_id"],
        "eghifjkffmcmffejmaaeicejpfopplem"
    );
}

#[tokio::test]
async fn start_rejects_non_bridge_http_port_without_killing_it() {
    let fake = TinyHttpServer::start("404 Not Found", "{\"error\":\"not bridge\"}");
    let app_dir = tempfile::tempdir().expect("temp app dir");
    fs::write(app_dir.path().join("version"), EXTENSION_VERSION).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tmwd-cdp-bridge"))
        .arg("start")
        .env("CDP_BRIDGE_APP_DIR", app_dir.path())
        .env("CDP_BRIDGE_WS_PORT", free_port().to_string())
        .env("CDP_BRIDGE_HTTP_PORT", fake.port.to_string())
        .output()
        .expect("run start with occupied http port");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("non tmwd-cdp-bridge process"));
    assert!(stderr.contains("CDP_BRIDGE_HTTP_PORT"));
    assert!(stderr.contains("not killing automatically"));

    let body = reqwest::get(format!("http://127.0.0.1:{}/health", fake.port))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("not bridge"));
}

#[tokio::test]
async fn start_rejects_existing_compatible_bridge_and_preserves_pid_file() {
    let bridge = BridgeProcess::start().await;
    let output = Command::new(env!("CARGO_BIN_EXE_tmwd-cdp-bridge"))
        .arg("start")
        .env("CDP_BRIDGE_APP_DIR", bridge.app_dir.path())
        .env("CDP_BRIDGE_WS_PORT", free_port().to_string())
        .env("CDP_BRIDGE_HTTP_PORT", bridge.http_port.to_string())
        .output()
        .expect("run start with existing bridge");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already running"));
    assert!(stderr.contains("reuse it"));
    assert_eq!(
        fs::read_to_string(bridge.app_dir.path().join("pid"))
            .unwrap()
            .trim()
            .parse::<u64>()
            .unwrap(),
        bridge.child.id() as u64
    );
}

#[tokio::test]
async fn start_clears_stale_pid_file() {
    let app_dir = tempfile::tempdir().expect("temp app dir");
    fs::write(app_dir.path().join("version"), EXTENSION_VERSION).unwrap();
    fs::write(app_dir.path().join("pid"), "999999").unwrap();
    let ws_port = free_port();
    let http_port = free_port();
    let child = Command::new(env!("CARGO_BIN_EXE_tmwd-cdp-bridge"))
        .arg("start")
        .env("CDP_BRIDGE_APP_DIR", app_dir.path())
        .env("CDP_BRIDGE_WS_PORT", ws_port.to_string())
        .env("CDP_BRIDGE_HTTP_PORT", http_port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn bridge");
    let bridge = BridgeProcess {
        child,
        app_dir,
        ws_port,
        http_port,
    };
    bridge.wait_for_health().await;
    assert_eq!(
        fs::read_to_string(bridge.app_dir.path().join("pid"))
            .unwrap()
            .trim()
            .parse::<u64>()
            .unwrap(),
        bridge.child.id() as u64
    );
}

#[tokio::test]
async fn stop_verifies_pid_and_releases_ports() {
    let mut bridge = BridgeProcess::start().await;
    let output = Command::new(env!("CARGO_BIN_EXE_tmwd-cdp-bridge"))
        .arg("stop")
        .arg("--json")
        .env("CDP_BRIDGE_APP_DIR", bridge.app_dir.path())
        .env("CDP_BRIDGE_WS_PORT", bridge.ws_port.to_string())
        .env("CDP_BRIDGE_HTTP_PORT", bridge.http_port.to_string())
        .output()
        .expect("run stop");
    assert!(output.status.success());
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["stopped"], true);
    assert_eq!(body["pid"], bridge.child.id());
    assert_eq!(body["http_port"], bridge.http_port);
    wait_until_unhealthy(&bridge.url("/health")).await;
    let _ = bridge.child.wait();
    assert!(!bridge.app_dir.path().join("pid").exists());
    let child = Command::new(env!("CARGO_BIN_EXE_tmwd-cdp-bridge"))
        .arg("start")
        .env("CDP_BRIDGE_APP_DIR", bridge.app_dir.path())
        .env("CDP_BRIDGE_WS_PORT", bridge.ws_port.to_string())
        .env("CDP_BRIDGE_HTTP_PORT", bridge.http_port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("restart bridge");
    bridge.child = child;
    bridge.wait_for_health().await;
}

#[tokio::test]
async fn stop_refuses_pid_mismatch() {
    let bridge = BridgeProcess::start().await;
    fs::write(bridge.app_dir.path().join("pid"), "1").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tmwd-cdp-bridge"))
        .arg("stop")
        .env("CDP_BRIDGE_APP_DIR", bridge.app_dir.path())
        .env("CDP_BRIDGE_WS_PORT", bridge.ws_port.to_string())
        .env("CDP_BRIDGE_HTTP_PORT", bridge.http_port.to_string())
        .output()
        .expect("run stop pid mismatch");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("pid file"));
    assert!(stderr.contains("does not match"));
    let client = reqwest::Client::new();
    assert!(
        client
            .get(bridge.url("/health"))
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
}

#[tokio::test]
async fn websocket_hello_grants_token_then_requires_auth_reconnect() {
    let bridge = BridgeProcess::start().await;

    let mut req = bridge.ws_url().into_client_request().unwrap();
    req.headers_mut()
        .insert("Origin", ALLOWED_EXTENSION_ORIGIN.parse().unwrap());
    let (mut ws, _) = connect_async(req).await.expect("ws connect");

    let msg = next_ws_json(&mut ws).await;
    assert_eq!(msg["type"], "auth_required");
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type":"hello"}).to_string(),
    ))
    .await
    .unwrap();
    let grant = next_ws_json(&mut ws).await;
    assert_eq!(grant["type"], "token_grant");
    assert_eq!(grant["token"], bridge.token());

    let client = reqwest::Client::new();
    let health: Value = client
        .get(bridge.url("/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["extension_connected"], false);

    let mut ws = connect_extension(&bridge).await;
    send_tabs(&mut ws, json!([])).await;
    let health: Value = client
        .get(bridge.url("/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["extension_connected"], true);
}

#[tokio::test]
async fn health_reports_extension_heartbeat_timestamps() {
    let bridge = BridgeProcess::start().await;
    let client = reqwest::Client::new();
    let mut ws = connect_extension(&bridge).await;

    let health: Value = client
        .get(bridge.url("/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["extension_connected"], true);
    let connected_at = health["extension_connected_at_unix_ms"]
        .as_u64()
        .expect("connected timestamp");
    let first_seen = health["extension_last_seen_at_unix_ms"]
        .as_u64()
        .expect("last seen timestamp");
    assert!(first_seen >= connected_at);
    assert!(health["extension_last_seen_age_ms"].as_u64().is_some());

    tokio::time::sleep(Duration::from_millis(5)).await;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type":"ping"}).to_string(),
    ))
    .await
    .unwrap();
    let health: Value = client
        .get(bridge.url("/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        health["extension_connected_at_unix_ms"].as_u64(),
        Some(connected_at)
    );
    assert!(health["extension_last_seen_at_unix_ms"].as_u64().unwrap() >= first_seen);

    ws.close(None).await.unwrap();
    for _ in 0..20 {
        let health: Value = client
            .get(bridge.url("/health"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if health["extension_connected"] == false {
            assert_eq!(health["extension_connected_at_unix_ms"], Value::Null);
            assert!(health["extension_last_seen_at_unix_ms"].as_u64().is_some());
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("extension did not disconnect");
}

#[tokio::test]
async fn websocket_auth_origin_tabs_and_execute_roundtrip_work() {
    let bridge = BridgeProcess::start().await;
    let client = reqwest::Client::new();

    let mut ws = connect_extension(&bridge).await;
    send_tabs(
        &mut ws,
        json!([{"id":7,"url":"https://example.com","title":"Example","active":true,"window_id":1}]),
    )
    .await;

    let http = tokio::spawn({
        let client = client.clone();
        let url = bridge.url("/v1/rpc");
        let token = bridge.token();
        async move {
            client
                .post(url)
                .bearer_auth(token)
                .json(&json!({
                        "cmd":"execute_js",
                        "request_id":"exec-1",
                        "code":"document.title",
                        "timeout":5
                }))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    });

    let exec = next_ws_json(&mut ws).await;
    assert_eq!(exec["id"], "exec-1");
    assert_eq!(exec["tabId"], 7);
    assert_eq!(exec["code"], "document.title");
    assert_eq!(exec["fallback"], "none");

    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type":"result","id":"exec-1","result":"Example","newTabs":[]}).to_string(),
    ))
    .await
    .unwrap();

    let response = http.await.unwrap();
    assert_eq!(response["r"]["request_id"], "exec-1");
    assert_eq!(response["r"]["data"], "Example");
}

#[tokio::test]
async fn execute_js_cdp_mode_and_fallback_are_forwarded_to_extension() {
    let bridge = BridgeProcess::start().await;
    let client = reqwest::Client::new();
    let mut ws = connect_extension(&bridge).await;
    send_tabs(
        &mut ws,
        json!([{"id":8,"url":"https://example.com/app","title":"App","active":true,"window_id":1}]),
    )
    .await;

    let cdp_http = tokio::spawn({
        let client = client.clone();
        let url = bridge.url("/v1/rpc");
        let token = bridge.token();
        async move {
            client
                .post(url)
                .bearer_auth(token)
                .json(&json!({
                    "cmd":"execute_js",
                    "request_id":"cdp-1",
                    "mode":"cdp",
                    "code":{
                        "method":"Runtime.evaluate",
                        "params":{"expression":"document.title","returnByValue":true}
                    }
                }))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    });
    let cdp_exec = next_ws_json(&mut ws).await;
    assert_eq!(cdp_exec["id"], "cdp-1");
    assert_eq!(cdp_exec["tabId"], 8);
    assert_eq!(
        cdp_exec["code"],
        json!({
            "cmd":"cdp",
            "method":"Runtime.evaluate",
            "params":{"expression":"document.title","returnByValue":true}
        })
    );
    assert_eq!(cdp_exec["fallback"], "none");
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type":"result","id":"cdp-1","result":{"result":{"value":"App"}},"newTabs":[]})
            .to_string(),
    ))
    .await
    .unwrap();
    let cdp_response = cdp_http.await.unwrap();
    assert_eq!(cdp_response["r"]["data"], json!({"result":{"value":"App"}}));

    let fallback_http = tokio::spawn({
        let client = client.clone();
        let url = bridge.url("/v1/rpc");
        let token = bridge.token();
        async move {
            client
                .post(url)
                .bearer_auth(token)
                .json(&json!({
                    "cmd":"execute_js",
                    "request_id":"fallback-1",
                    "fallback":"cdp",
                    "code":"document.title"
                }))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    });
    let fallback_exec = next_ws_json(&mut ws).await;
    assert_eq!(fallback_exec["id"], "fallback-1");
    assert_eq!(fallback_exec["code"], "document.title");
    assert_eq!(fallback_exec["fallback"], "cdp");
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type":"result","id":"fallback-1","result":"App","newTabs":[]}).to_string(),
    ))
    .await
    .unwrap();
    let fallback_response = fallback_http.await.unwrap();
    assert_eq!(fallback_response["r"]["data"], "App");
}

#[tokio::test]
async fn execute_timeout_cleans_pending_and_allows_follow_up_request() {
    let bridge = BridgeProcess::start().await;
    let client = reqwest::Client::new();
    let mut ws = connect_extension(&bridge).await;
    send_tabs(
        &mut ws,
        json!([{"id":31,"url":"https://example.com","title":"Example","active":true,"window_id":1}]),
    )
    .await;

    let timed_out = tokio::spawn({
        let client = client.clone();
        let url = bridge.url("/v1/rpc");
        let token = bridge.token();
        async move {
            client
                .post(url)
                .bearer_auth(token)
                .json(&json!({
                    "cmd":"execute_js",
                    "request_id":"timeout-1",
                    "code":"document.title",
                    "timeout":1
                }))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    });

    let exec = next_ws_json(&mut ws).await;
    assert_eq!(exec["id"], "timeout-1");
    let timed_out = timed_out.await.unwrap();
    assert_eq!(timed_out["r"]["request_id"], "timeout-1");
    assert_eq!(timed_out["r"]["error"]["code"], "EXEC_TIMEOUT");

    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type":"result","id":"timeout-1","result":"late","newTabs":[]}).to_string(),
    ))
    .await
    .unwrap();

    let follow_up = tokio::spawn({
        let client = client.clone();
        let url = bridge.url("/v1/rpc");
        let token = bridge.token();
        async move {
            client
                .post(url)
                .bearer_auth(token)
                .json(&json!({
                    "cmd":"execute_js",
                    "request_id":"after-timeout",
                    "code":"document.title",
                    "timeout":5
                }))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    });

    let exec = next_ws_json(&mut ws).await;
    assert_eq!(exec["id"], "after-timeout");
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type":"result","id":"after-timeout","result":"Example","newTabs":[]}).to_string(),
    ))
    .await
    .unwrap();

    let follow_up = follow_up.await.unwrap();
    assert_eq!(follow_up["r"]["data"], "Example");
}

#[tokio::test]
async fn extension_disconnect_returns_exec_error_and_does_not_hang() {
    let bridge = BridgeProcess::start().await;
    let client = reqwest::Client::new();
    let mut ws = connect_extension(&bridge).await;
    send_tabs(
        &mut ws,
        json!([{"id":41,"url":"https://example.com","title":"Example","active":true,"window_id":1}]),
    )
    .await;

    let http = tokio::spawn({
        let client = client.clone();
        let url = bridge.url("/v1/rpc");
        let token = bridge.token();
        async move {
            client
                .post(url)
                .bearer_auth(token)
                .json(&json!({
                    "cmd":"execute_js",
                    "request_id":"disconnect-1",
                    "code":"document.title",
                    "timeout":10
                }))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    });

    let exec = next_ws_json(&mut ws).await;
    assert_eq!(exec["id"], "disconnect-1");
    ws.close(None).await.unwrap();

    let response = http.await.unwrap();
    assert_eq!(response["r"]["request_id"], "disconnect-1");
    assert_eq!(response["r"]["error"]["code"], "EXEC_ERROR");
    assert!(
        response["r"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("extension disconnected")
    );
}

#[tokio::test]
async fn find_session_filters_by_url_and_title() {
    let bridge = BridgeProcess::start().await;
    let client = reqwest::Client::new();
    let mut ws = connect_extension(&bridge).await;
    send_tabs(
        &mut ws,
        json!([
            {"id":10,"url":"https://alpha.example/dashboard","title":"Alpha Dashboard","active":false,"window_id":1},
            {"id":11,"url":"https://beta.example/reports","title":"Beta Reports","active":true,"window_id":1},
            {"id":12,"url":"chrome://extensions","title":"Extensions","active":false,"window_id":1}
        ]),
    )
    .await;

    let found = rpc(
        &client,
        &bridge,
        json!({
            "cmd":"find_session",
            "request_id":"find-beta",
            "url_contains":"beta.example",
            "title_contains":"Reports"
        }),
    )
    .await;
    assert_eq!(found["r"]["request_id"], "find-beta");
    assert_eq!(found["r"]["data"]["id"], 11);
    assert_eq!(found["r"]["data"]["title"], "Beta Reports");

    let missing = rpc(
        &client,
        &bridge,
        json!({
            "cmd":"find_session",
            "request_id":"find-missing",
            "url_contains":"alpha.example",
            "title_contains":"Reports"
        }),
    )
    .await;
    assert_eq!(missing["r"]["request_id"], "find-missing");
    assert_eq!(missing["r"]["error"]["code"], "NO_SESSION");
}

#[tokio::test]
async fn batch_executes_items_in_order_and_preserves_item_errors() {
    let bridge = BridgeProcess::start().await;
    let client = reqwest::Client::new();
    let mut ws = connect_extension(&bridge).await;
    send_tabs(
        &mut ws,
        json!([{"id":21,"url":"https://example.com","title":"Example","active":true,"window_id":1}]),
    )
    .await;

    let http = tokio::spawn({
        let client = client.clone();
        let url = bridge.url("/v1/rpc");
        let token = bridge.token();
        async move {
            client
                .post(url)
                .bearer_auth(token)
                .json(&json!({
                    "cmd":"batch",
                    "request_id":"batch-1",
                    "items":[
                        {"cmd":"execute_js","request_id":"item-1","code":"throw new Error('boom')"},
                        {"cmd":"execute_js","request_id":"item-2","code":"document.title"}
                    ]
                }))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    });

    let first = next_ws_json(&mut ws).await;
    assert_eq!(first["id"], "item-1");
    assert_eq!(first["code"], "throw new Error('boom')");
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type":"error","id":"item-1","error":{"message":"boom"}}).to_string(),
    ))
    .await
    .unwrap();

    let second = next_ws_json(&mut ws).await;
    assert_eq!(second["id"], "item-2");
    assert_eq!(second["code"], "document.title");
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type":"result","id":"item-2","result":"Example","newTabs":[]}).to_string(),
    ))
    .await
    .unwrap();

    let response = http.await.unwrap();
    assert_eq!(response["r"]["request_id"], "batch-1");
    assert_eq!(response["r"]["items"][0]["request_id"], "item-1");
    assert_eq!(response["r"]["items"][0]["error"]["code"], "EXEC_ERROR");
    assert_eq!(response["r"]["items"][0]["error"]["message"], "boom");
    assert_eq!(response["r"]["items"][1]["request_id"], "item-2");
    assert_eq!(response["r"]["items"][1]["data"], "Example");
}

#[tokio::test]
async fn websocket_rejects_wrong_origin() {
    let bridge = BridgeProcess::start().await;
    let mut req = bridge.ws_url().into_client_request().unwrap();
    req.headers_mut()
        .insert("Origin", "https://evil.example".parse().unwrap());
    let err = connect_async(req).await.unwrap_err();
    assert!(err.to_string().contains("403") || err.to_string().contains("HTTP"));

    let mut req = bridge.ws_url().into_client_request().unwrap();
    req.headers_mut().insert(
        "Origin",
        "chrome-extension://aikfggdiblmijobpgdapacebmcjknbof"
            .parse()
            .unwrap(),
    );
    let err = connect_async(req).await.unwrap_err();
    assert!(err.to_string().contains("403") || err.to_string().contains("HTTP"));
}

#[test]
fn install_command_copies_extension_and_version() {
    let app_dir = tempfile::tempdir().expect("temp app dir");
    let output = Command::new(env!("CARGO_BIN_EXE_tmwd-cdp-bridge"))
        .arg("install")
        .arg("chrome")
        .env("CDP_BRIDGE_APP_DIR", app_dir.path())
        .output()
        .expect("run install");
    assert!(output.status.success());

    let extension_dir = app_dir.path().join("extension");
    assert!(extension_dir.join("manifest.json").is_file());
    assert!(extension_dir.join("background.js").is_file());
    assert_eq!(
        fs::read_to_string(app_dir.path().join("version")).unwrap(),
        "2.0"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("chrome://extensions"));
    assert!(stdout.contains(extension_dir.to_string_lossy().as_ref()));
}

#[cfg(unix)]
#[tokio::test]
async fn token_file_is_private_on_unix() {
    use std::os::unix::fs::PermissionsExt;

    let bridge = BridgeProcess::start().await;
    let mode = fs::metadata(bridge.app_dir.path().join("token"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}
