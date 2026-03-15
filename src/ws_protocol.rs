use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Client → Server request
#[derive(Debug, Deserialize)]
pub struct WsRequest {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// Server → Client response (success or error)
#[derive(Debug, Serialize)]
pub struct WsResponse {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WsError>,
}

#[derive(Debug, Serialize)]
pub struct WsError {
    pub code: i32,
    pub message: String,
}

/// Server → Client push event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsEvent {
    pub event: String,
    pub data: Value,
}

impl WsResponse {
    pub fn ok(id: u64, result: Value) -> Self {
        WsResponse {
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, code: i32, message: impl Into<String>) -> Self {
        WsResponse {
            id,
            result: None,
            error: Some(WsError {
                code,
                message: message.into(),
            }),
        }
    }
}
