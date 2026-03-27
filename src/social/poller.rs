#![allow(dead_code)]

// Polling logic for Instagram and Threads.
// Called by scheduler.rs on the configured interval.
// Uses social_cursors table to track last-fetched ID per (platform, feed).

use crate::config::{InstagramConfig, ThreadsConfig};
use crate::error::Result;
use crate::social::instagram::InstagramClient;
use crate::social::threads::ThreadsClient;
use crate::social::{SocialItem, SocialPlatform};
use crate::state::StateDb;
use tracing::{debug, warn};

/// Poll all configured Instagram feeds, return new SocialItems.
pub async fn poll_instagram(cfg: &InstagramConfig, db: &StateDb) -> Result<Vec<SocialItem>> {
    let token = resolve_env(&cfg.token_env)?;
    let client = InstagramClient::new(token, cfg.user_id.clone());
    let mut items = Vec::new();

    for feed in &cfg.subscribe {
        match feed.as_str() {
            "comments" => {
                let new_items = poll_ig_comments(&client, db).await?;
                items.extend(new_items);
            }
            "mentions" => {
                let new_items = poll_ig_mentions(&client, db).await?;
                items.extend(new_items);
            }
            "messages" => {} // DM — webhook only, no polling support
            unknown => {
                warn!(feed = unknown, "instagram: unknown subscribe feed, skipping");
            }
        }
    }
    Ok(items)
}

/// Poll all configured Threads feeds, return new SocialItems.
pub async fn poll_threads(cfg: &ThreadsConfig, db: &StateDb) -> Result<Vec<SocialItem>> {
    let token = resolve_env(&cfg.token_env)?;
    let client = ThreadsClient::new(token, cfg.user_id.clone());
    let mut items = Vec::new();

    for feed in &cfg.subscribe {
        match feed.as_str() {
            "replies" => {
                let new_items = poll_th_replies(&client, db).await?;
                items.extend(new_items);
            }
            "mentions" => {
                let new_items = poll_th_mentions(&client, db).await?;
                items.extend(new_items);
            }
            unknown => {
                warn!(feed = unknown, "threads: unknown subscribe feed, skipping");
            }
        }
    }
    Ok(items)
}

// ── Instagram pollers ─────────────────────────────────────────────────────────

async fn poll_ig_comments(client: &InstagramClient, db: &StateDb) -> Result<Vec<SocialItem>> {
    let cursor = db.get_social_cursor("instagram", "comments")?;
    // Fetch recent media (up to 10 posts) then their comments.
    let media = client.get_media(10).await?;
    let media_list = media
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut items = Vec::new();
    let mut newest_id: Option<String> = cursor.clone();

    for media_obj in &media_list {
        let media_id = match media_obj.get("id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => continue,
        };
        let comments = client.get_comments(media_id, None).await?;
        let comment_list = comments
            .get("data")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        for comment in &comment_list {
            let id = match comment.get("id").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => continue,
            };
            // Skip already-seen IDs (lexicographic comparison on numeric ID strings).
            if let Some(ref last) = cursor {
                if id.as_str() <= last.as_str() {
                    continue;
                }
            }
            if newest_id.as_deref().map(|n| id.as_str() > n).unwrap_or(true) {
                newest_id = Some(id.clone());
            }
            items.push(SocialItem {
                platform: SocialPlatform::Instagram,
                platform_id: id,
                event_type: "comment".to_string(),
                author_id: comment
                    .get("from")
                    .and_then(|f| f.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                author_name: comment
                    .get("username")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                media_id: Some(media_id.to_string()),
                text: comment
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                metadata: comment.clone(),
            });
        }
    }

    if let Some(ref nid) = newest_id {
        if cursor.as_deref() != Some(nid.as_str()) {
            db.upsert_social_cursor("instagram", "comments", nid)?;
        }
    }

    debug!("instagram poll comments: {} new items", items.len());
    Ok(items)
}

async fn poll_ig_mentions(client: &InstagramClient, db: &StateDb) -> Result<Vec<SocialItem>> {
    // Instagram @mentions of a Business Account are only reliably delivered via
    // webhook (field: "mentions"). The Graph API does not expose a dedicated
    // polling edge for incoming mentions on other users' posts.
    //
    // This polling path returns an empty list — configure mode="webhook" and
    // subscribe to the "mentions" field to receive these events in real time.
    //
    // NOTE: If you need to poll mentions of YOUR OWN posts' captions, use the
    // /me/tags edge (requires instagram_manage_comments permission).
    let _ = (client, db); // suppress unused warnings
    debug!("instagram poll mentions: not supported in polling mode (use webhook)");
    Ok(Vec::new())
}

// ── Threads pollers ───────────────────────────────────────────────────────────

async fn poll_th_replies(client: &ThreadsClient, db: &StateDb) -> Result<Vec<SocialItem>> {
    let cursor = db.get_social_cursor("threads", "replies")?;
    let timeline = client.get_timeline(20).await?;
    let posts = timeline
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut items = Vec::new();
    let mut newest_id: Option<String> = cursor.clone();

    for post in &posts {
        let post_id = match post.get("id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => continue,
        };
        let replies = client.get_replies(post_id, None).await?;
        let reply_list = replies
            .get("data")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        for reply in &reply_list {
            let id = match reply.get("id").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => continue,
            };
            if let Some(ref last) = cursor {
                if id.as_str() <= last.as_str() {
                    continue;
                }
            }
            if newest_id.as_deref().map(|n| id.as_str() > n).unwrap_or(true) {
                newest_id = Some(id.clone());
            }
            items.push(SocialItem {
                platform: SocialPlatform::Threads,
                platform_id: id,
                event_type: "reply".to_string(),
                author_id: reply
                    .get("username")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                author_name: reply
                    .get("username")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                media_id: Some(post_id.to_string()),
                text: reply
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                metadata: reply.clone(),
            });
        }
    }

    if let Some(ref nid) = newest_id {
        if cursor.as_deref() != Some(nid.as_str()) {
            db.upsert_social_cursor("threads", "replies", nid)?;
        }
    }

    debug!("threads poll replies: {} new items", items.len());
    Ok(items)
}

async fn poll_th_mentions(client: &ThreadsClient, db: &StateDb) -> Result<Vec<SocialItem>> {
    // Threads @mentions are only reliably delivered via webhook (field: "mentions").
    // There is no dedicated polling edge for incoming mentions on other users' posts.
    // Configure mode="webhook" and subscribe to "mentions" to receive these events.
    let _ = (client, db);
    debug!("threads poll mentions: not supported in polling mode (use webhook)");
    Ok(Vec::new())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn resolve_env(env_var: &str) -> Result<String> {
    std::env::var(env_var).map_err(|_| {
        crate::error::CatClawError::Social(format!(
            "env var '{}' not set — required for polling",
            env_var
        ))
    })
}
