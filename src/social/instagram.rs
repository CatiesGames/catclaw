#![allow(dead_code)]

use reqwest::Client;
use serde_json::Value;
use crate::error::{CatClawError, Result};

/// Instagram Graph API client.
/// Supports both Facebook Login tokens (EAA... → graph.facebook.com)
/// and Instagram Login tokens (IG... → graph.instagram.com).
pub struct InstagramClient {
    token: String,
    user_id: String,
    http: Client,
}

/// Detect whether a token was issued via Instagram Login (prefix IG) vs Facebook Login (prefix EAA).
fn is_ig_login_token(token: &str) -> bool {
    token.starts_with("IG")
}

impl InstagramClient {
    pub fn new(token: String, user_id: String) -> Self {
        Self {
            token,
            user_id,
            http: Client::new(),
        }
    }

    fn base(&self) -> &'static str {
        if is_ig_login_token(&self.token) {
            "https://graph.instagram.com/v25.0"
        } else {
            "https://graph.facebook.com/v25.0"
        }
    }

    pub async fn get_profile(&self) -> Result<Value> {
        let url = format!(
            "{}/{}?fields=id,name,username,followers_count,media_count",
            self.base(), self.user_id,
        );
        self.get(&url).await
    }

    pub async fn get_media(&self, limit: u32) -> Result<Value> {
        let url = format!(
            "{}/{}/media?fields=id,caption,media_type,timestamp,permalink,like_count,comments_count&limit={}",
            self.base(), self.user_id, limit,
        );
        self.get(&url).await
    }

    /// Fetch a single media/post by ID (caption, permalink, etc.).
    pub async fn get_media_by_id(&self, media_id: &str) -> Result<Value> {
        let url = format!(
            "{}/{}?fields=id,caption,media_type,timestamp,permalink",
            self.base(), media_id,
        );
        self.get(&url).await
    }

    pub async fn get_comments(&self, media_id: &str, since_id: Option<&str>) -> Result<Value> {
        let mut url = format!(
            "{}/{}/comments?fields=id,text,username,timestamp,from",
            self.base(), media_id,
        );
        if let Some(sid) = since_id {
            url.push_str(&format!("&after={}", sid));
        }
        self.get(&url).await
    }

    /// Reply to a comment.
    pub async fn reply_comment(&self, comment_id: &str, message: &str) -> Result<Value> {
        let url = format!("{}/{}/replies", self.base(), comment_id);
        self.post_form(&url, &[("message", message)]).await
    }

    pub async fn get_mentioned_media(&self, mention_id: &str) -> Result<Value> {
        let url = format!(
            "{}/{}?fields=mentioned_media.fields(id,caption,media_url,permalink)&mention_id={}",
            self.base(), self.user_id, mention_id,
        );
        self.get(&url).await
    }

    pub async fn delete_comment(&self, comment_id: &str) -> Result<Value> {
        let url = format!("{}/{}", self.base(), comment_id);
        self.delete(&url).await
    }

    /// Create a new image post (two-step: create container → publish).
    pub async fn create_image_post(&self, image_url: &str, caption: &str) -> Result<Value> {
        // Step 1: create media container
        let container_url = format!("{}/{}/media", self.base(), self.user_id);
        let container: Value = self
            .post_form(&container_url, &[("image_url", image_url), ("caption", caption)])
            .await?;
        let container_id = container
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CatClawError::Social("instagram: no container id in response".into()))?
            .to_string();

        // Step 2: wait for container processing, then publish.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let publish_url = format!("{}/{}/media_publish", self.base(), self.user_id);
        self.post_form(&publish_url, &[("creation_id", &container_id)]).await
    }

    /// Create a carousel post with multiple images (three-step: child containers → carousel container → publish).
    /// `image_urls` must contain 2–10 publicly accessible JPEG URLs.
    pub async fn create_carousel_post(&self, image_urls: &[&str], caption: &str) -> Result<Value> {
        if image_urls.len() < 2 {
            return Err(CatClawError::Social("carousel requires at least 2 images".into()));
        }
        if image_urls.len() > 10 {
            return Err(CatClawError::Social("carousel supports at most 10 images".into()));
        }

        // Step 1: create child containers (one per image, no caption).
        let container_url = format!("{}/{}/media", self.base(), self.user_id);
        let mut child_ids = Vec::with_capacity(image_urls.len());
        for url in image_urls {
            let child: Value = self
                .post_form(&container_url, &[("image_url", *url), ("is_carousel_item", "true")])
                .await?;
            let child_id = child
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CatClawError::Social("instagram: no child container id".into()))?
                .to_string();
            child_ids.push(child_id);
        }

        // Step 2: wait for child containers to finish processing.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;

        // Step 3: create carousel container.
        let children_csv = child_ids.join(",");
        let carousel: Value = self
            .post_form(
                &container_url,
                &[("media_type", "CAROUSEL"), ("children", &children_csv), ("caption", caption)],
            )
            .await?;
        let carousel_id = carousel
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CatClawError::Social("instagram: no carousel container id".into()))?
            .to_string();

        // Step 4: wait for carousel container processing, then publish.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let publish_url = format!("{}/{}/media_publish", self.base(), self.user_id);
        self.post_form(&publish_url, &[("creation_id", &carousel_id)]).await
    }

    /// Send a direct message to a user.
    pub async fn send_dm(&self, recipient_id: &str, text: &str) -> Result<Value> {
        let url = format!("{}/me/messages", self.base());
        let body = serde_json::json!({
            "recipient": { "id": recipient_id },
            "message": { "text": text }
        });
        let resp = self.http.post(&url)
            .bearer_auth(&self.token)
            .json(&body)
            .send().await
            .map_err(|e| CatClawError::Social(format!("instagram http error: {e}")))?;
        let val: Value = resp.json().await
            .map_err(|e| CatClawError::Social(format!("instagram json error: {e}")))?;
        check_error(val)
    }

    // ── Token management ──────────────────────────────────────────────────────

    /// Check how many seconds until the token expires.
    ///
    /// - Facebook Login tokens: uses `debug_token` endpoint (returns exact expiry).
    /// - Instagram Login tokens: no debug_token support; returns 60*86400 (assumed 60-day lifetime).
    /// - Returns 0 if the token never expires (System User Token).
    pub async fn check_token_expiry(token: &str) -> Result<i64> {
        let http = Client::new();

        if is_ig_login_token(token) {
            // IG Login tokens don't support debug_token.
            // Verify the token is valid by calling /me, then assume 60-day lifetime.
            let resp = http.get("https://graph.instagram.com/me")
                .query(&[("fields", "id"), ("access_token", token)])
                .send().await
                .map_err(|e| CatClawError::Social(format!("instagram http error: {e}")))?;
            let val: Value = resp.json().await
                .map_err(|e| CatClawError::Social(format!("instagram json error: {e}")))?;
            check_error(val)?;
            // Token is valid — no exact expiry available, return 60 days as estimate.
            // The scheduler's 30-day refresh cadence ensures renewal well before expiry.
            Ok(60 * 86400)
        } else {
            // Facebook Login tokens: use debug_token for exact expiry.
            let resp = http.get("https://graph.facebook.com/v25.0/debug_token")
                .query(&[("input_token", token), ("access_token", token)])
                .send().await
                .map_err(|e| CatClawError::Social(format!("instagram http error: {e}")))?;
            let val: Value = resp.json().await
                .map_err(|e| CatClawError::Social(format!("instagram json error: {e}")))?;
            let val = check_error(val)?;
            let expires_in = val
                .get("data")
                .and_then(|d| d.get("expires_at"))
                .and_then(|v| v.as_i64())
                .map(|expires_at| {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;
                    expires_at - now
                })
                .unwrap_or(0);
            Ok(expires_in)
        }
    }

    /// Exchange a short-lived token for a long-lived token (60 days).
    /// Detects token type and uses the correct endpoint/grant_type.
    pub async fn exchange_token(app_id: &str, app_secret: &str, short_token: &str) -> Result<String> {
        let http = Client::new();
        let (url, params): (&str, Vec<(&str, &str)>) = if is_ig_login_token(short_token) {
            ("https://graph.instagram.com/access_token", vec![
                ("grant_type", "ig_exchange_token"),
                ("client_secret", app_secret),
                ("access_token", short_token),
            ])
        } else {
            ("https://graph.facebook.com/v25.0/oauth/access_token", vec![
                ("grant_type", "fb_exchange_token"),
                ("client_id", app_id),
                ("client_secret", app_secret),
                ("fb_exchange_token", short_token),
            ])
        };
        let resp = http.get(url)
            .query(&params)
            .send().await
            .map_err(|e| CatClawError::Social(format!("instagram http error: {e}")))?;
        let val: Value = resp.json().await
            .map_err(|e| CatClawError::Social(format!("instagram json error: {e}")))?;
        let val = check_error(val)?;
        val.get("access_token")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| CatClawError::Social("instagram: no access_token in exchange response".into()))
    }

    /// Refresh a long-lived token before it expires (returns a new long-lived token).
    pub async fn refresh_token(token: &str) -> Result<String> {
        let http = Client::new();
        let url = if is_ig_login_token(token) {
            "https://graph.instagram.com/refresh_access_token"
        } else {
            "https://graph.facebook.com/v25.0/oauth/access_token"
        };
        let resp = http.get(url)
            .query(&[
                ("grant_type", "ig_refresh_token"),
                ("access_token", token),
            ])
            .send().await
            .map_err(|e| CatClawError::Social(format!("instagram http error: {e}")))?;
        let val: Value = resp.json().await
            .map_err(|e| CatClawError::Social(format!("instagram json error: {e}")))?;
        let val = check_error(val)?;
        val.get("access_token")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| CatClawError::Social("instagram: no access_token in refresh response".into()))
    }

    pub async fn get_insights(&self, metric: &str, period: &str) -> Result<Value> {
        let url = format!(
            "{}/{}/insights?metric={}&period={}",
            self.base(), self.user_id, metric, period,
        );
        self.get(&url).await
    }

    // ── HTTP helpers ──────────────────────────────────────────────────────────

    async fn get(&self, url: &str) -> Result<Value> {
        let resp = self.http.get(url)
            .bearer_auth(&self.token)
            .send().await
            .map_err(|e| CatClawError::Social(format!("instagram http error: {e}")))?;
        let val: Value = resp.json().await
            .map_err(|e| CatClawError::Social(format!("instagram json error: {e}")))?;
        check_error(val)
    }

    async fn post_form(&self, url: &str, params: &[(&str, &str)]) -> Result<Value> {
        let resp = self.http.post(url)
            .bearer_auth(&self.token)
            .form(params)
            .send().await
            .map_err(|e| CatClawError::Social(format!("instagram http error: {e}")))?;
        let val: Value = resp.json().await
            .map_err(|e| CatClawError::Social(format!("instagram json error: {e}")))?;
        check_error(val)
    }

    async fn delete(&self, url: &str) -> Result<Value> {
        let resp = self.http.delete(url)
            .bearer_auth(&self.token)
            .send().await
            .map_err(|e| CatClawError::Social(format!("instagram http error: {e}")))?;
        let val: Value = resp.json().await
            .map_err(|e| CatClawError::Social(format!("instagram json error: {e}")))?;
        check_error(val)
    }
}

fn check_error(val: Value) -> Result<Value> {
    if let Some(err) = val.get("error") {
        return Err(CatClawError::Social(format!(
            "instagram api error: {}",
            err
        )));
    }
    Ok(val)
}
