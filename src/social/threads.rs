#![allow(dead_code)]

use reqwest::Client;
use serde_json::Value;
use crate::error::{CatClawError, Result};

/// Threads API client. Token is a Threads OAuth token (60-day, needs periodic refresh).
pub struct ThreadsClient {
    token: String,
    user_id: String,
    http: Client,
}

impl ThreadsClient {
    pub fn new(token: String, user_id: String) -> Self {
        Self {
            token,
            user_id,
            http: Client::new(),
        }
    }

    fn base(&self) -> &'static str {
        "https://graph.threads.net/v1.0"
    }

    pub async fn get_profile(&self) -> Result<Value> {
        let url = format!(
            "{}/{}?fields=id,name,username,threads_profile_picture_url,threads_biography",
            self.base(), self.user_id,
        );
        self.get(&url).await
    }

    pub async fn get_timeline(&self, limit: u32) -> Result<Value> {
        let url = format!(
            "{}/{}/threads?fields=id,text,media_type,timestamp,permalink,like_count,replies_count&limit={}",
            self.base(), self.user_id, limit,
        );
        self.get(&url).await
    }

    pub async fn get_replies(&self, post_id: &str, since_id: Option<&str>) -> Result<Value> {
        let mut url = format!(
            "{}/{}/replies?fields=id,text,username,timestamp",
            self.base(), post_id,
        );
        if let Some(sid) = since_id {
            url.push_str(&format!("&after={}", sid));
        }
        self.get(&url).await
    }

    /// Create a new post (two-step: create container → publish).
    pub async fn create_post(&self, text: &str) -> Result<Value> {
        // Step 1: create container
        let container_url = format!("{}/{}/threads", self.base(), self.user_id);
        let container: Value = self
            .post_form(&container_url, &[("text", text), ("media_type", "TEXT")])
            .await?;
        let container_id = container
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CatClawError::Social("threads: no container id in response".into()))?
            .to_string();

        // Step 2: wait for container to finish processing, then publish.
        // Official docs recommend ~30 seconds between container creation and publish.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let publish_url = format!("{}/{}/threads_publish", self.base(), self.user_id);
        self.post_form(&publish_url, &[("creation_id", &container_id)]).await
    }

    /// Reply to a post (two-step: create reply container → publish).
    pub async fn reply(&self, post_id: &str, text: &str) -> Result<Value> {
        let container_url = format!("{}/{}/threads", self.base(), self.user_id);
        let container: Value = self
            .post_form(
                &container_url,
                &[("text", text), ("media_type", "TEXT"), ("reply_to_id", post_id)],
            )
            .await?;
        let container_id = container
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CatClawError::Social("threads: no container id in response".into()))?
            .to_string();

        // Wait for container processing before publishing.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let publish_url = format!("{}/{}/threads_publish", self.base(), self.user_id);
        self.post_form(&publish_url, &[("creation_id", &container_id)]).await
    }

    pub async fn delete_post(&self, post_id: &str) -> Result<Value> {
        let url = format!("{}/{}", self.base(), post_id);
        self.delete_req(&url).await
    }

    pub async fn get_insights(&self, metric: &str) -> Result<Value> {
        let url = format!(
            "{}/{}/threads_insights?metric={}",
            self.base(), self.user_id, metric,
        );
        self.get(&url).await
    }

    // ── HTTP helpers ──────────────────────────────────────────────────────────

    async fn get(&self, url: &str) -> Result<Value> {
        let resp = self.http.get(url)
            .bearer_auth(&self.token)
            .send().await
            .map_err(|e| CatClawError::Social(format!("threads http error: {e}")))?;
        let val: Value = resp.json().await
            .map_err(|e| CatClawError::Social(format!("threads json error: {e}")))?;
        check_error(val)
    }

    async fn post_form(&self, url: &str, params: &[(&str, &str)]) -> Result<Value> {
        let resp = self.http.post(url)
            .bearer_auth(&self.token)
            .form(params)
            .send().await
            .map_err(|e| CatClawError::Social(format!("threads http error: {e}")))?;
        let val: Value = resp.json().await
            .map_err(|e| CatClawError::Social(format!("threads json error: {e}")))?;
        check_error(val)
    }

    async fn delete_req(&self, url: &str) -> Result<Value> {
        let resp = self.http.delete(url)
            .bearer_auth(&self.token)
            .send().await
            .map_err(|e| CatClawError::Social(format!("threads http error: {e}")))?;
        let val: Value = resp.json().await
            .map_err(|e| CatClawError::Social(format!("threads json error: {e}")))?;
        check_error(val)
    }
}

fn check_error(val: Value) -> Result<Value> {
    if let Some(err) = val.get("error") {
        return Err(CatClawError::Social(format!("threads api error: {}", err)));
    }
    Ok(val)
}
