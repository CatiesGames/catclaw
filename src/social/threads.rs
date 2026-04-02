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

    /// Fetch a single post by ID (text, permalink, etc.).
    pub async fn get_post_by_id(&self, post_id: &str) -> Result<Value> {
        let url = format!(
            "{}/{}?fields=id,text,media_type,timestamp,permalink",
            self.base(), post_id,
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

    /// Create a new image post (two-step: create container → publish).
    /// `image_url` must be a publicly accessible HTTPS URL.
    pub async fn create_image_post(&self, image_url: &str, text: &str) -> Result<Value> {
        // Step 1: create container with media_type=IMAGE
        let container_url = format!("{}/{}/threads", self.base(), self.user_id);
        let container: Value = self
            .post_form(
                &container_url,
                &[("text", text), ("media_type", "IMAGE"), ("image_url", image_url)],
            )
            .await?;
        let container_id = container
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CatClawError::Social("threads: no container id in response".into()))?
            .to_string();

        // Step 2: wait for container to finish processing, then publish.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let publish_url = format!("{}/{}/threads_publish", self.base(), self.user_id);
        self.post_form(&publish_url, &[("creation_id", &container_id)]).await
    }

    /// Create a carousel post with multiple images (three-step: child containers → carousel container → publish).
    /// `image_urls` must contain 2–20 publicly accessible image URLs.
    pub async fn create_carousel_post(&self, image_urls: &[&str], text: &str) -> Result<Value> {
        if image_urls.len() < 2 {
            return Err(CatClawError::Social("carousel requires at least 2 images".into()));
        }
        if image_urls.len() > 20 {
            return Err(CatClawError::Social("carousel supports at most 20 images".into()));
        }

        // Step 1: create child containers (one per image).
        let container_url = format!("{}/{}/threads", self.base(), self.user_id);
        let mut child_ids = Vec::with_capacity(image_urls.len());
        for url in image_urls {
            let child: Value = self
                .post_form(
                    &container_url,
                    &[("media_type", "IMAGE"), ("image_url", *url), ("is_carousel_item", "true")],
                )
                .await?;
            let child_id = child
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CatClawError::Social("threads: no child container id".into()))?
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
                &[("media_type", "CAROUSEL"), ("children", &children_csv), ("text", text)],
            )
            .await?;
        let carousel_id = carousel
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CatClawError::Social("threads: no carousel container id".into()))?
            .to_string();

        // Step 4: wait for carousel container processing, then publish.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let publish_url = format!("{}/{}/threads_publish", self.base(), self.user_id);
        self.post_form(&publish_url, &[("creation_id", &carousel_id)]).await
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

    /// Search Threads posts by keyword.
    pub async fn keyword_search(&self, q: &str, search_type: Option<&str>, limit: Option<u32>) -> Result<Value> {
        let resp = self.http.get("https://graph.threads.net/v1.0/keyword_search")
            .bearer_auth(&self.token)
            .query(&[
                ("q", q),
                ("search_type", search_type.unwrap_or("TOP")),
                ("fields", "id,text,media_type,timestamp,permalink,username"),
            ])
            .query(&[("limit", limit.unwrap_or(25))])
            .send().await
            .map_err(|e| CatClawError::Social(format!("threads http error: {e}")))?;
        let val: Value = resp.json().await
            .map_err(|e| CatClawError::Social(format!("threads json error: {e}")))?;
        check_error(val)
    }

    pub async fn delete_post(&self, post_id: &str) -> Result<Value> {
        let url = format!("{}/{}", self.base(), post_id);
        self.delete_req(&url).await
    }

    // ── Token management ──────────────────────────────────────────────────────

    /// Exchange a short-lived Threads token for a long-lived token (60 days).
    pub async fn exchange_token(app_id: &str, app_secret: &str, short_token: &str) -> Result<String> {
        let http = Client::new();
        let resp = http.get("https://graph.threads.net/access_token")
            .query(&[
                ("grant_type", "th_exchange_token"),
                ("client_id", app_id),
                ("client_secret", app_secret),
                ("access_token", short_token),
            ])
            .send().await
            .map_err(|e| CatClawError::Social(format!("threads http error: {e}")))?;
        let val: Value = resp.json().await
            .map_err(|e| CatClawError::Social(format!("threads json error: {e}")))?;
        let val = check_error(val)?;
        val.get("access_token")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| CatClawError::Social("threads: no access_token in exchange response".into()))
    }

    /// Refresh a long-lived Threads token (returns a new long-lived token).
    pub async fn refresh_token(token: &str) -> Result<String> {
        let http = Client::new();
        let resp = http.get("https://graph.threads.net/refresh_access_token")
            .query(&[
                ("grant_type", "th_refresh_token"),
                ("access_token", token),
            ])
            .send().await
            .map_err(|e| CatClawError::Social(format!("threads http error: {e}")))?;
        let val: Value = resp.json().await
            .map_err(|e| CatClawError::Social(format!("threads json error: {e}")))?;
        let val = check_error(val)?;
        val.get("access_token")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| CatClawError::Social("threads: no access_token in refresh response".into()))
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
