#![allow(dead_code)]

use reqwest::Client;
use serde_json::Value;
use crate::error::{CatClawError, Result};

/// Instagram Graph API client using a System User Token (never expires).
pub struct InstagramClient {
    token: String,
    user_id: String,
    http: Client,
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
        "https://graph.facebook.com/v25.0"
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
