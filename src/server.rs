use std::{fs, net::SocketAddr};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{
        State,
        rejection::JsonRejection,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::{net::TcpListener, signal, time::Duration};
use tracing::{error, info, warn};

use crate::{
    auth,
    config::{ALLOWED_EXTENSION_ID, BridgeConfig},
    protocol::{BatchResult, ErrorCode, RpcEnvelope, RpcRequest, RpcResult},
    session::{BridgeState, ExtensionResponse, TabInfo},
};

#[derive(Clone)]
pub struct AppState {
    pub bridge: BridgeState,
    pub token: String,
    pub version: String,
    pub pid: u32,
    pub allowed_extension_origin: String,
}

pub async fn run_server(config: BridgeConfig) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tmwd_cdp_bridge=info,tmwd_cdp_bridge=info".into()),
        )
        .init();
    config.ensure_app_dir()?;
    config.validate_installed_extension_version()?;
    preflight_http_port(&config).await?;
    clear_stale_pid_file(&config)?;
    let token = auth::load_or_create_token(&config.token_path())?;
    fs::write(config.pid_path(), std::process::id().to_string())?;
    info!("Bearer token: {}", auth::token_prefix(&token));

    let state = AppState {
        bridge: BridgeState::new(),
        token,
        version: env!("CARGO_PKG_VERSION").to_string(),
        pid: std::process::id(),
        allowed_extension_origin: config.allowed_extension_origin.clone(),
    };

    let ws_addr = SocketAddr::from(([127, 0, 0, 1], config.ws_port));
    let http_addr = SocketAddr::from(([127, 0, 0, 1], config.http_port));
    let ws_listener = TcpListener::bind(ws_addr)
        .await
        .with_context(|| {
            format!(
                "WS port {} is in use. Set CDP_BRIDGE_WS_PORT or stop that process; not killing automatically.",
                config.ws_port
            )
        })?;
    let http_listener = TcpListener::bind(http_addr)
        .await
        .with_context(|| {
            format!(
                "HTTP port {} is in use. Set CDP_BRIDGE_HTTP_PORT or stop that process; not killing automatically.",
                config.http_port
            )
        })?;

    let ws_app = Router::new()
        .route("/", get(ws_handler))
        .with_state(state.clone());
    let http_app = Router::new()
        .route("/health", get(health))
        .route("/v1/rpc", post(rpc))
        .with_state(state);

    info!("WS listening on {ws_addr}");
    info!("HTTP listening on {http_addr}");

    let ws_server = axum::serve(ws_listener, ws_app).with_graceful_shutdown(shutdown_signal());
    let http_server =
        axum::serve(http_listener, http_app).with_graceful_shutdown(shutdown_signal());
    tokio::try_join!(ws_server, http_server)?;
    Ok(())
}

async fn preflight_http_port(config: &BridgeConfig) -> Result<()> {
    let url = format!("http://127.0.0.1:{}/health", config.http_port);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(700))
        .build()?;
    let Ok(response) = client.get(&url).send().await else {
        return Ok(());
    };
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!(
            "HTTP port {} is in use by a non tmwd-cdp-bridge process: GET /health returned {status}. Set CDP_BRIDGE_HTTP_PORT or stop that process; not killing automatically.",
            config.http_port
        );
    }
    let body: Value = response
        .json()
        .await
        .context("parse existing /health response")?;
    if body.get("server").and_then(Value::as_str) != Some("tmwd-cdp-bridge") {
        anyhow::bail!(
            "HTTP port {} is in use by a non tmwd-cdp-bridge process. Set CDP_BRIDGE_HTTP_PORT or stop that process; not killing automatically.",
            config.http_port
        );
    }
    let existing_version = body
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if existing_version != env!("CARGO_PKG_VERSION") {
        anyhow::bail!(
            "tmwd-cdp-bridge already running on HTTP port {} with incompatible version {existing_version}; expected {}. Run 'tmwd-cdp-bridge stop' for that installation or choose another CDP_BRIDGE_HTTP_PORT.",
            config.http_port,
            env!("CARGO_PKG_VERSION")
        );
    }
    let pid = body.get("pid").and_then(Value::as_u64);
    anyhow::bail!(
        "tmwd-cdp-bridge already running on HTTP port {}{}; reuse it or run 'tmwd-cdp-bridge stop' first.",
        config.http_port,
        pid.map(|pid| format!(" (pid {pid})")).unwrap_or_default()
    );
}

fn clear_stale_pid_file(config: &BridgeConfig) -> Result<()> {
    match fs::remove_file(config.pid_path()) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => {
            Err(err).with_context(|| format!("remove stale pid {}", config.pid_path().display()))
        }
    }
}

async fn shutdown_signal() {
    let _ = signal::ctrl_c().await;
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let connection = state.bridge.connection_status().await;
    Json(json!({
        "server": "tmwd-cdp-bridge",
        "version": state.version,
        "pid": state.pid,
        "extension_id": ALLOWED_EXTENSION_ID,
        "allowed_extension_origin": state.allowed_extension_origin,
        "extension_connected": connection.connected,
        "extension_connected_at_unix_ms": connection.connected_at_unix_ms,
        "extension_last_seen_at_unix_ms": connection.last_seen_at_unix_ms,
        "extension_last_seen_age_ms": connection.last_seen_age_ms,
    }))
}

async fn ws_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let origin = headers
        .get("origin")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    if origin != state.allowed_extension_origin {
        warn!("rejecting websocket origin: {origin}");
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.on_upgrade(move |socket| handle_socket(state, socket))
}

async fn handle_socket(state: AppState, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    if sink
        .send(Message::Text(json!({"type":"auth_required"}).to_string()))
        .await
        .is_err()
    {
        return;
    }
    let mut sink = Some(sink);
    let mut authenticated = false;
    while let Some(msg) = stream.next().await {
        match msg {
            Ok(Message::Text(text)) if !authenticated => {
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    break;
                };
                match value.get("type").and_then(Value::as_str) {
                    Some("hello") => {
                        let Some(sink) = sink.as_mut() else {
                            break;
                        };
                        let _ = sink
                            .send(Message::Text(
                                json!({"type":"token_grant","token":state.token}).to_string(),
                            ))
                            .await;
                        break;
                    }
                    Some("auth")
                        if value.get("token").and_then(Value::as_str) == Some(&state.token) =>
                    {
                        let Some(mut authed_sink) = sink.take() else {
                            break;
                        };
                        if authed_sink
                            .send(Message::Text(json!({"type":"auth_ok"}).to_string()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                        state.bridge.attach(authed_sink).await;
                        authenticated = true;
                        info!("extension authenticated");
                    }
                    Some("auth") => {
                        let Some(sink) = sink.as_mut() else {
                            break;
                        };
                        let _ = sink
                            .send(Message::Text(json!({"type":"auth_error"}).to_string()))
                            .await;
                        break;
                    }
                    _ => break,
                }
            }
            Ok(Message::Text(text)) => handle_ws_text(&state, &text).await,
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(err) => {
                error!("websocket error: {err}");
                break;
            }
        }
    }
    if authenticated {
        state.bridge.detach().await;
        info!("extension disconnected");
    }
}

async fn handle_ws_text(state: &AppState, text: &str) {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return;
    };
    state.bridge.mark_seen().await;
    match value.get("type").and_then(Value::as_str) {
        Some("ext_ready") | Some("tabs_update") => {
            let tabs = value
                .get("tabs")
                .cloned()
                .and_then(|v| serde_json::from_value::<Vec<TabInfo>>(v).ok())
                .unwrap_or_default();
            state.bridge.update_tabs(tabs).await;
        }
        Some("result") | Some("error") => {
            if let Some(id) = value.get("id").and_then(Value::as_str) {
                let ok = value.get("type").and_then(Value::as_str) == Some("result");
                let response = ExtensionResponse {
                    ok,
                    result: value.get("result").cloned(),
                    error: value.get("error").cloned(),
                    new_tabs: value
                        .get("newTabs")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default(),
                };
                state.bridge.complete(id, response).await;
            }
        }
        Some("ping") | Some("ack") => {}
        _ => {}
    }
}

async fn rpc(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: Result<Json<RpcRequest>, JsonRejection>,
) -> impl IntoResponse {
    let req = match req {
        Ok(Json(req)) => req,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(RpcEnvelope {
                    r: RpcResult::err(
                        state.bridge.next_request_id(),
                        ErrorCode::BadRequest,
                        err.to_string(),
                    ),
                }),
            )
                .into_response();
        }
    };
    if !authorized(&headers, &state.token) {
        let id = req
            .request_id
            .unwrap_or_else(|| state.bridge.next_request_id());
        return (
            StatusCode::UNAUTHORIZED,
            Json(RpcEnvelope {
                r: RpcResult::err(id, ErrorCode::Unauthorized, "invalid bearer token"),
            }),
        )
            .into_response();
    }
    if req.cmd == "batch" {
        let result = handle_batch(&state, req).await;
        return Json(RpcEnvelope { r: result }).into_response();
    }
    let result = handle_single(&state, req).await;
    Json(RpcEnvelope { r: result }).into_response()
}

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        == Some(token)
}

async fn handle_batch(state: &AppState, req: RpcRequest) -> BatchResult {
    let batch_id = req
        .request_id
        .unwrap_or_else(|| state.bridge.next_request_id());
    let mut items = Vec::with_capacity(req.items.len());
    for item in req.items {
        items.push(handle_single(state, item).await);
    }
    BatchResult {
        request_id: batch_id,
        items,
    }
}

async fn handle_single(state: &AppState, req: RpcRequest) -> RpcResult {
    let request_id = req
        .request_id
        .clone()
        .unwrap_or_else(|| state.bridge.next_request_id());
    match req.cmd.as_str() {
        "execute_js" => execute_js(state, req, request_id).await,
        "get_all_sessions" => {
            let tabs = state.bridge.all_tabs().await;
            RpcResult::ok(request_id, json!(tabs), Vec::new())
        }
        "find_session" => {
            let tabs = state.bridge.all_tabs().await;
            let found = tabs.into_iter().find(|tab| {
                req.url_contains
                    .as_ref()
                    .is_none_or(|needle| tab.url.contains(needle))
                    && req
                        .title_contains
                        .as_ref()
                        .is_none_or(|needle| tab.title.contains(needle))
                    && req.browser.as_ref().is_none_or(|_| true)
            });
            match found {
                Some(tab) => RpcResult::ok(request_id, json!(tab), Vec::new()),
                None => RpcResult::err(request_id, ErrorCode::NoSession, "no matching session"),
            }
        }
        "shutdown" => {
            tokio::spawn(async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                std::process::exit(0);
            });
            RpcResult::ok(request_id, json!({"shutting_down":true}), Vec::new())
        }
        _ => RpcResult::err(request_id, ErrorCode::BadRequest, "unknown cmd"),
    }
}

async fn execute_js(state: &AppState, req: RpcRequest, request_id: String) -> RpcResult {
    if !state.bridge.is_connected().await {
        return RpcResult::err(
            request_id,
            ErrorCode::NoExtension,
            "没有已连接的扩展，请运行 status 检查",
        );
    }
    let session_id = req
        .session_id
        .clone()
        .or_else(|| req.tab_id.map(|id| id.to_string()));
    let tab = match state.bridge.select_session(session_id.as_deref()).await {
        Ok(tab) => tab,
        Err(err) => return RpcResult::err(request_id, ErrorCode::NoSession, format!("{err:#}")),
    };
    let code = match bridge_code(&req) {
        Ok(code) => code,
        Err(err) => return RpcResult::err(request_id, ErrorCode::BadRequest, err),
    };
    let timeout_secs = req.timeout.unwrap_or(15);
    match state
        .bridge
        .send_execute(
            request_id.clone(),
            tab.id,
            code,
            req.fallback.clone(),
            timeout_secs,
        )
        .await
    {
        Ok(response) if response.ok => RpcResult::ok(
            request_id,
            response.result.unwrap_or(Value::Null),
            response.new_tabs,
        ),
        Ok(response) => RpcResult::err(
            request_id,
            ErrorCode::ExecError,
            response
                .error
                .map(error_message)
                .unwrap_or_else(|| "extension execution failed".to_string()),
        ),
        Err(err) => {
            let code = if err.to_string().contains("timed out") {
                ErrorCode::ExecTimeout
            } else {
                ErrorCode::Internal
            };
            RpcResult::err(request_id, code, format!("{err:#}"))
        }
    }
}

fn bridge_code(req: &RpcRequest) -> Result<Value, String> {
    match req.fallback.as_deref().unwrap_or("none") {
        "none" | "cdp" => {}
        other => return Err(format!("unsupported fallback: {other}")),
    }
    let Some(code) = req.code.clone() else {
        return Err("execute_js requires code".to_string());
    };
    if req.mode.as_deref() == Some("cdp") {
        let obj = code
            .as_object()
            .ok_or_else(|| "mode=cdp requires command object".to_string())?;
        let method = obj
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| "mode=cdp requires method".to_string())?;
        return Ok(json!({
            "cmd": "cdp",
            "method": method,
            "params": obj.get("params").cloned().unwrap_or_else(|| json!({})),
        }));
    }
    Ok(code)
}

fn error_message(value: Value) -> String {
    value
        .get("message")
        .or_else(|| value.get("msg"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_code_accepts_only_documented_fallbacks() {
        let req = RpcRequest {
            cmd: "execute_js".into(),
            request_id: None,
            session_id: None,
            tab_id: None,
            code: Some(json!("document.title")),
            mode: None,
            fallback: Some("cdp".into()),
            timeout: None,
            items: Vec::new(),
            url_contains: None,
            title_contains: None,
            browser: None,
        };
        assert_eq!(bridge_code(&req).unwrap(), json!("document.title"));

        let req = RpcRequest {
            fallback: Some("auto".into()),
            ..req
        };
        assert!(
            bridge_code(&req)
                .unwrap_err()
                .contains("unsupported fallback")
        );
    }
}
