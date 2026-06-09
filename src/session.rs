use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use axum::extract::ws::Message;
use futures_util::{SinkExt, stream::SplitSink};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    sync::{Mutex, oneshot},
    time::{Duration, timeout},
};

type WsSink = SplitSink<axum::extract::ws::WebSocket, Message>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub id: u64,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub window_id: Option<u64>,
}

#[derive(Debug)]
struct Pending {
    sender: oneshot::Sender<ExtensionResponse>,
}

#[derive(Debug)]
pub struct ExtensionResponse {
    pub ok: bool,
    pub result: Option<Value>,
    pub error: Option<Value>,
    pub new_tabs: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionStatus {
    pub connected: bool,
    pub connected_at_unix_ms: Option<u64>,
    pub last_seen_at_unix_ms: Option<u64>,
    pub last_seen_age_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct BridgeState {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    ws_sink: Mutex<Option<WsSink>>,
    tabs: Mutex<Vec<TabInfo>>,
    last_used: Mutex<Option<u64>>,
    connected_at_unix_ms: Mutex<Option<u64>>,
    last_seen_at_unix_ms: Mutex<Option<u64>>,
    pending: Mutex<HashMap<String, Pending>>,
    next_id: AtomicU64,
}

impl BridgeState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                ws_sink: Mutex::new(None),
                tabs: Mutex::new(Vec::new()),
                last_used: Mutex::new(None),
                connected_at_unix_ms: Mutex::new(None),
                last_seen_at_unix_ms: Mutex::new(None),
                pending: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(1),
            }),
        }
    }

    pub fn next_request_id(&self) -> String {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        format!("req-{id}")
    }

    pub async fn attach(&self, sink: WsSink) {
        let now = now_unix_ms();
        *self.inner.ws_sink.lock().await = Some(sink);
        *self.inner.connected_at_unix_ms.lock().await = Some(now);
        *self.inner.last_seen_at_unix_ms.lock().await = Some(now);
    }

    pub async fn detach(&self) {
        *self.inner.ws_sink.lock().await = None;
        *self.inner.connected_at_unix_ms.lock().await = None;
        let mut pending = self.inner.pending.lock().await;
        for (_, p) in pending.drain() {
            let _ = p.sender.send(ExtensionResponse {
                ok: false,
                result: None,
                error: Some(json!({"message":"extension disconnected"})),
                new_tabs: Vec::new(),
            });
        }
    }

    pub async fn is_connected(&self) -> bool {
        self.inner.ws_sink.lock().await.is_some()
    }

    pub async fn mark_seen(&self) {
        *self.inner.last_seen_at_unix_ms.lock().await = Some(now_unix_ms());
    }

    pub async fn connection_status(&self) -> ConnectionStatus {
        let connected = self.is_connected().await;
        let connected_at_unix_ms = *self.inner.connected_at_unix_ms.lock().await;
        let last_seen_at_unix_ms = *self.inner.last_seen_at_unix_ms.lock().await;
        let last_seen_age_ms =
            last_seen_at_unix_ms.map(|last_seen| now_unix_ms().saturating_sub(last_seen));
        ConnectionStatus {
            connected,
            connected_at_unix_ms,
            last_seen_at_unix_ms,
            last_seen_age_ms,
        }
    }

    pub async fn update_tabs(&self, tabs: Vec<TabInfo>) {
        *self.inner.tabs.lock().await = tabs;
    }

    pub async fn all_tabs(&self) -> Vec<TabInfo> {
        self.inner.tabs.lock().await.clone()
    }

    pub async fn select_session(&self, requested: Option<&str>) -> Result<TabInfo> {
        let tabs = self.inner.tabs.lock().await.clone();
        if let Some(id) = requested {
            let parsed = id.parse::<u64>().context("sessionId must be numeric")?;
            return tabs
                .into_iter()
                .find(|t| t.id == parsed)
                .context("specified session does not exist");
        }
        if let Some(tab) = tabs
            .iter()
            .find(|t| t.active && is_scriptable(&t.url))
            .cloned()
        {
            return Ok(tab);
        }
        if let Some(last) = *self.inner.last_used.lock().await
            && let Some(tab) = tabs
                .iter()
                .find(|t| t.id == last && is_scriptable(&t.url))
                .cloned()
        {
            return Ok(tab);
        }
        tabs.into_iter()
            .find(|t| is_scriptable(&t.url))
            .context("no usable browser session")
    }

    pub async fn send_execute(
        &self,
        id: String,
        tab_id: u64,
        code: Value,
        fallback: Option<String>,
        timeout_secs: u64,
    ) -> Result<ExtensionResponse> {
        let (tx, rx) = oneshot::channel();
        self.inner
            .pending
            .lock()
            .await
            .insert(id.clone(), Pending { sender: tx });
        let payload = json!({
            "id": id,
            "tabId": tab_id,
            "code": code,
            "fallback": fallback.unwrap_or_else(|| "none".to_string()),
        });
        let send_result = async {
            let mut guard = self.inner.ws_sink.lock().await;
            let sink = guard
                .as_mut()
                .context("no authenticated extension connected")?;
            sink.send(Message::Text(payload.to_string())).await?;
            anyhow::Ok(())
        }
        .await;
        if let Err(err) = send_result {
            self.inner.pending.lock().await.remove(&id);
            return Err(err);
        }
        let response = timeout(Duration::from_secs(timeout_secs), rx).await;
        self.inner.pending.lock().await.remove(&id);
        let response = response.context("request timed out")??;
        if response.ok {
            *self.inner.last_used.lock().await = Some(tab_id);
        }
        Ok(response)
    }

    pub async fn complete(&self, id: &str, response: ExtensionResponse) {
        if let Some(pending) = self.inner.pending.lock().await.remove(id) {
            let _ = pending.sender.send(response);
        }
    }
}

impl Default for BridgeState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn is_scriptable(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn select_session_prefers_active_then_last_then_first_scriptable() {
        let state = BridgeState::new();
        state
            .update_tabs(vec![
                TabInfo {
                    id: 1,
                    url: "chrome://extensions".into(),
                    title: "Extensions".into(),
                    active: true,
                    window_id: None,
                },
                TabInfo {
                    id: 2,
                    url: "https://example.com".into(),
                    title: "Example".into(),
                    active: false,
                    window_id: None,
                },
                TabInfo {
                    id: 3,
                    url: "https://active.example".into(),
                    title: "Active".into(),
                    active: true,
                    window_id: None,
                },
            ])
            .await;
        assert_eq!(state.select_session(None).await.unwrap().id, 3);
        assert_eq!(state.select_session(Some("2")).await.unwrap().id, 2);
    }
}
