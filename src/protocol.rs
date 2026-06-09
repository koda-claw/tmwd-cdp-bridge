use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    pub cmd: String,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default, rename = "sessionId")]
    pub session_id: Option<String>,
    #[serde(default, rename = "tabId")]
    pub tab_id: Option<u64>,
    #[serde(default)]
    pub code: Option<Value>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub fallback: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub items: Vec<RpcRequest>,
    #[serde(default)]
    pub url_contains: Option<String>,
    #[serde(default)]
    pub title_contains: Option<String>,
    #[serde(default)]
    pub browser: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RpcEnvelope<T: Serialize> {
    pub r: T,
}

#[derive(Debug, Serialize)]
pub struct RpcResult {
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "newTabs")]
    pub new_tabs: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
pub struct BatchResult {
    pub request_id: String,
    pub items: Vec<RpcResult>,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[allow(dead_code)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    Unauthorized,
    BadRequest,
    NoExtension,
    NoSession,
    TabClosed,
    ExecTimeout,
    ExecError,
    CdpUnavailable,
    PortInUse,
    Internal,
}

impl RpcResult {
    pub fn ok(request_id: String, data: Value, new_tabs: Vec<Value>) -> Self {
        Self {
            request_id,
            data: Some(data),
            new_tabs,
            error: None,
        }
    }

    pub fn err(request_id: String, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            request_id,
            data: None,
            new_tabs: Vec::new(),
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
}
