// Webhook handlers for Instagram and Threads Meta webhook events.
// Mounted by gateway.rs onto the axum Router.
//
// GET  /webhook/instagram  — hub verification (hub.challenge echo)
// POST /webhook/instagram  — event delivery (HMAC-SHA256 verified)
// GET  /webhook/threads    — hub verification
// POST /webhook/threads    — event delivery

use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::gateway::GatewayHandle;
use crate::social::SocialItem;

// ── Router builder ────────────────────────────────────────────────────────────

/// Build axum webhook routes, sharing the GatewayHandle state.
/// Only mounts routes for configured platforms.
pub fn build_router(gw: Arc<GatewayHandle>) -> Router<Arc<GatewayHandle>> {
    let cfg = gw.config.read().unwrap();
    let has_ig = cfg.social.instagram.is_some();
    let has_th = cfg.social.threads.is_some();
    drop(cfg);

    let mut router: Router<Arc<GatewayHandle>> = Router::new();
    if has_ig {
        router = router
            .route("/webhook/instagram", get(verify_instagram))
            .route("/webhook/instagram", post(receive_instagram));
    }
    if has_th {
        router = router
            .route("/webhook/threads", get(verify_threads))
            .route("/webhook/threads", post(receive_threads));
    }
    router
}

// ── Hub verification ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct HubParams {
    #[serde(rename = "hub.mode")]
    pub mode: Option<String>,
    #[serde(rename = "hub.verify_token")]
    pub verify_token: Option<String>,
    #[serde(rename = "hub.challenge")]
    pub challenge: Option<String>,
}

pub async fn verify_instagram(
    Query(params): Query<HubParams>,
    State(gw): State<Arc<GatewayHandle>>,
) -> impl IntoResponse {
    let token = {
        let cfg = gw.config.read().unwrap();
        cfg.social.instagram.as_ref()
            .and_then(|c| c.webhook_verify_token_env.as_ref())
            .and_then(|env| std::env::var(env).ok())
    };
    handle_verify(params, token.as_deref())
}

pub async fn verify_threads(
    Query(params): Query<HubParams>,
    State(gw): State<Arc<GatewayHandle>>,
) -> impl IntoResponse {
    let token = {
        let cfg = gw.config.read().unwrap();
        cfg.social.threads.as_ref()
            .and_then(|c| c.webhook_verify_token_env.as_ref())
            .and_then(|env| std::env::var(env).ok())
    };
    handle_verify(params, token.as_deref())
}

fn handle_verify(params: HubParams, expected_token: Option<&str>) -> impl IntoResponse {
    if params.mode.as_deref() != Some("subscribe") {
        return (StatusCode::FORBIDDEN, "invalid mode".to_string());
    }
    let Some(challenge) = params.challenge else {
        return (StatusCode::BAD_REQUEST, "missing challenge".to_string());
    };
    match (params.verify_token.as_deref(), expected_token) {
        (Some(got), Some(expected)) if got == expected => (StatusCode::OK, challenge),
        _ => (StatusCode::FORBIDDEN, "token mismatch".to_string()),
    }
}

// ── Event delivery ────────────────────────────────────────────────────────────

pub async fn receive_instagram(
    headers: HeaderMap,
    State(gw): State<Arc<GatewayHandle>>,
    body: Bytes,
) -> impl IntoResponse {
    let (mode, secret) = {
        let cfg = gw.config.read().unwrap();
        let mode = cfg.social.instagram.as_ref().map(|c| c.mode.clone()).unwrap_or_default();
        let secret = cfg.social.instagram.as_ref()
            .and_then(|c| c.app_secret_env.as_ref())
            .and_then(|env| std::env::var(env).ok());
        (mode, secret)
    };
    // Silently accept but ignore events when not in webhook mode.
    // Meta requires a 200 response or it will retry indefinitely.
    if mode != "webhook" {
        return StatusCode::OK;
    }
    if let Some(ref s) = secret {
        if !verify_signature(&headers, &body, s) {
            warn!("instagram webhook: invalid HMAC signature");
            return StatusCode::FORBIDDEN;
        }
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
        warn!("instagram webhook: failed to parse body");
        return StatusCode::BAD_REQUEST;
    };
    let items = parse_instagram_events(&payload);
    for item in items {
        let _ = gw.social_item_tx.send(item);
    }
    StatusCode::OK
}

pub async fn receive_threads(
    headers: HeaderMap,
    State(gw): State<Arc<GatewayHandle>>,
    body: Bytes,
) -> impl IntoResponse {
    let (mode, secret) = {
        let cfg = gw.config.read().unwrap();
        let mode = cfg.social.threads.as_ref().map(|c| c.mode.clone()).unwrap_or_default();
        let secret = cfg.social.threads.as_ref()
            .and_then(|c| c.app_secret_env.as_ref())
            .and_then(|env| std::env::var(env).ok());
        (mode, secret)
    };
    if mode != "webhook" {
        return StatusCode::OK;
    }
    if let Some(ref s) = secret {
        if !verify_signature(&headers, &body, s) {
            warn!("threads webhook: invalid HMAC signature");
            return StatusCode::FORBIDDEN;
        }
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
        warn!("threads webhook: failed to parse body");
        return StatusCode::BAD_REQUEST;
    };
    let items = parse_threads_events(&payload);
    for item in items {
        let _ = gw.social_item_tx.send(item);
    }
    StatusCode::OK
}

// ── HMAC-SHA256 verification ──────────────────────────────────────────────────

fn verify_signature(headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let sig_header = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let expected = if let Some(s) = sig_header.strip_prefix("sha256=") {
        s
    } else {
        return false;
    };

    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(body);
    let result = hex::encode(mac.finalize().into_bytes());
    // Constant-time comparison via timing-safe equality on hex strings.
    result.len() == expected.len()
        && result
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

// ── Event parsers ─────────────────────────────────────────────────────────────

fn parse_instagram_events(payload: &Value) -> Vec<SocialItem> {
    use crate::social::SocialPlatform;
    let mut out = Vec::new();

    let entries = match payload.get("entry").and_then(|v| v.as_array()) {
        Some(e) => e,
        None => {
            debug!("instagram webhook: no entry array");
            return out;
        }
    };

    for entry in entries {
        let changes = match entry.get("changes").and_then(|v| v.as_array()) {
            Some(c) => c,
            None => continue,
        };
        for change in changes {
            let field = change.get("field").and_then(|v| v.as_str()).unwrap_or("");
            let value = match change.get("value") {
                Some(v) => v,
                None => continue,
            };
            let event_type = match field {
                "comments" => "comment",
                "mentions" => "mention",
                "messages" => "message",
                _ => field,
            };
            let platform_id = value
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    // Fallback: combine timestamp + field
                    format!(
                        "{}:{}",
                        field,
                        value.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0)
                    )
                });
            out.push(SocialItem {
                platform: SocialPlatform::Instagram,
                platform_id,
                event_type: event_type.to_string(),
                author_id: value
                    .get("from")
                    .and_then(|f| f.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                author_name: value
                    .get("from")
                    .and_then(|f| f.get("username"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                media_id: value
                    .get("media")
                    .and_then(|m| m.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                text: value
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                metadata: value.clone(),
            });
        }
    }
    out
}

fn parse_threads_events(payload: &Value) -> Vec<SocialItem> {
    use crate::social::SocialPlatform;
    let mut out = Vec::new();

    let entries = match payload.get("entry").and_then(|v| v.as_array()) {
        Some(e) => e,
        None => {
            debug!("threads webhook: no entry array");
            return out;
        }
    };

    for entry in entries {
        let changes = match entry.get("changes").and_then(|v| v.as_array()) {
            Some(c) => c,
            None => continue,
        };
        for change in changes {
            let field = change.get("field").and_then(|v| v.as_str()).unwrap_or("");
            let value = match change.get("value") {
                Some(v) => v,
                None => continue,
            };
            let event_type = match field {
                "replies" => "reply",
                "mentions" => "mention",
                "quotes" => "quote",
                "publish" => "publish",
                "delete" => "delete",
                _ => field,
            };
            let platform_id = value
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    format!(
                        "{}:{}",
                        field,
                        value.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0)
                    )
                });
            out.push(SocialItem {
                platform: SocialPlatform::Threads,
                platform_id,
                event_type: event_type.to_string(),
                author_id: value
                    .get("from")
                    .and_then(|f| f.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                author_name: value
                    .get("from")
                    .and_then(|f| f.get("username"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                media_id: value
                    .get("replied_to")
                    .and_then(|m| m.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                text: value
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                metadata: value.clone(),
            });
        }
    }
    out
}
