use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tracing::warn;

use crate::ws_protocol::{WsEvent, WsResponse};

/// WebSocket client for connecting TUI to a remote Gateway.
pub struct GatewayClient {
    /// Send raw JSON text to the WS write task
    write_tx: mpsc::UnboundedSender<String>,
    /// Pending request waiters: id → oneshot sender
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<WsResponse>>>>,
    /// Counter for generating unique request IDs
    next_id: AtomicU64,
}

impl GatewayClient {
    /// Connect to a Gateway WebSocket server.
    /// Returns the client + an event receiver for push events.
    pub async fn connect(
        url: &str,
        token: &str,
    ) -> std::result::Result<
        (Arc<Self>, mpsc::UnboundedReceiver<WsEvent>),
        Box<dyn std::error::Error + Send + Sync>,
    > {
        // Retry WS handshake a few times — gateway may accept TCP before WS is ready
        let mut last_err = None;
        let mut ws_stream_opt = None;
        for _ in 0..10 {
            match tokio_tungstenite::connect_async(url).await {
                Ok((stream, _)) => {
                    ws_stream_opt = Some(stream);
                    break;
                }
                Err(e) => {
                    last_err = Some(e);
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
        }
        let mut ws_stream = ws_stream_opt.ok_or_else(|| {
            last_err.unwrap_or_else(|| tokio_tungstenite::tungstenite::Error::ConnectionClosed)
        })?;

        // Send auth token as first message
        if !token.is_empty() {
            let auth_msg = serde_json::json!({"auth": token}).to_string();
            ws_stream.send(Message::Text(auth_msg.into())).await?;
        }
        let (ws_sink, mut ws_read) = ws_stream.split();

        let (write_tx, mut write_rx) = mpsc::unbounded_channel::<String>();
        let (event_tx, event_rx) = mpsc::unbounded_channel::<WsEvent>();
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<WsResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Writer task
        tokio::spawn(async move {
            let mut sink = ws_sink;
            while let Some(text) = write_rx.recv().await {
                if sink.send(Message::Text(text)).await.is_err() {
                    break;
                }
            }
        });

        // Reader task: dispatch responses to waiting oneshots, events to event channel
        let pending_clone = pending.clone();
        let event_tx_clone = event_tx.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_read.next().await {
                let text = match msg {
                    Message::Text(t) => t,
                    Message::Close(_) => break,
                    _ => continue,
                };

                // Try as WsResponse (has "id" field)
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    if v.get("id").is_some() {
                        // It's a response
                        if let Ok(resp) = serde_json::from_str::<ResponseRaw>(&text) {
                            let mut map = pending_clone.lock().await;
                            if let Some(tx) = map.remove(&resp.id) {
                                let ws_resp = if let Some(err) = resp.error {
                                    WsResponse::err(resp.id, err.code, err.message)
                                } else {
                                    WsResponse::ok(resp.id, resp.result.unwrap_or(Value::Null))
                                };
                                let _ = tx.send(ws_resp);
                            }
                        }
                    } else if v.get("event").is_some() {
                        // It's a push event
                        if let Ok(event) = serde_json::from_str::<WsEvent>(&text) {
                            let _ = event_tx_clone.send(event);
                        }
                    }
                }
            }
            warn!("WS read loop ended — connection closed");
        });

        let client = Arc::new(GatewayClient {
            write_tx,
            pending,
            next_id: AtomicU64::new(1),
        });

        Ok((client, event_rx))
    }

    /// Send a request and wait for the response.
    pub async fn request(
        &self,
        method: &str,
        params: Value,
    ) -> std::result::Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = json!({ "id": id, "method": method, "params": params });
        let text = serde_json::to_string(&req).map_err(|e| e.to_string())?;

        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.pending.lock().await;
            map.insert(id, tx);
        }

        if self.write_tx.send(text).is_err() {
            self.pending.lock().await.remove(&id);
            return Err("WS connection closed".to_string());
        }

        let resp = rx.await.map_err(|_| "response channel dropped".to_string())?;

        if let Some(err) = resp.error {
            Err(err.message)
        } else {
            Ok(resp.result.unwrap_or(Value::Null))
        }
    }

    /// Check if the connection is still alive
    #[allow(dead_code)]
    pub fn is_connected(&self) -> bool {
        !self.write_tx.is_closed()
    }
}

/// Raw response for deserialization (mirrors WsResponse but with Deserialize)
#[derive(serde::Deserialize)]
struct ResponseRaw {
    id: u64,
    result: Option<Value>,
    error: Option<ErrorRaw>,
}

#[derive(serde::Deserialize)]
struct ErrorRaw {
    code: i32,
    message: String,
}
