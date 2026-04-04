#![allow(dead_code)]

// Polling logic for Instagram and Threads.
// Called by scheduler.rs on the configured interval.
// Uses social_cursors table to track last-fetched timestamp per (platform, feed).
//
// IMPORTANT: Meta platform IDs are NOT monotonically increasing — a newer post can
// have a smaller numeric ID than an older one. Cursors must use timestamps, not IDs.

use crate::config::{InstagramConfig, ThreadsConfig};
use crate::error::Result;
use crate::social::instagram::InstagramClient;
use crate::social::threads::ThreadsClient;
use crate::social::{SocialItem, SocialPlatform};
use crate::state::StateDb;
use tracing::{debug, warn};

/// Compare two ISO 8601 timestamp strings. Returns true if `a` is strictly after `b`.
fn ts_gt(a: &str, b: &str) -> bool {
    a > b
}

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
    debug!(cursor = ?cursor, "instagram poll comments: starting");

    // Fetch recent media (up to 10 posts) then their comments.
    let media = client.get_media(10).await?;
    let media_list = media
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    debug!("instagram poll: fetched {} media posts", media_list.len());

    let mut items = Vec::new();
    let mut newest_ts: Option<String> = cursor.clone();
    let mut total_comments = 0u32;
    let mut skipped_by_cursor = 0u32;
    let mut skipped_by_dedup = 0u32;

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

        debug!(media_id, comments = comment_list.len(), "instagram poll: media comments");

        for comment in &comment_list {
            let id = match comment.get("id").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => continue,
            };
            let ts = comment
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            total_comments += 1;
            if let Some(ref last_ts) = cursor {
                if !ts_gt(&ts, last_ts) {
                    skipped_by_cursor += 1;
                    continue;
                }
            }
            // Check dedup — already in inbox?
            if db.get_social_inbox_by_platform_id("instagram", &id).ok().flatten().is_some() {
                skipped_by_dedup += 1;
                continue;
            }
            if newest_ts.as_deref().map(|n| ts_gt(&ts, n)).unwrap_or(true) {
                newest_ts = Some(ts.clone());
            }
            let username = comment
                .get("username")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let text = comment
                .get("text")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            debug!(id = %id, ts = %ts, username = ?username, text = ?text, "instagram poll: new comment");
            items.push(SocialItem {
                platform: SocialPlatform::Instagram,
                platform_id: id,
                event_type: "comment".to_string(),
                author_id: comment
                    .get("from")
                    .and_then(|f| f.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                author_name: username,
                media_id: Some(media_id.to_string()),
                text,
                metadata: comment.clone(),
            });
        }
    }

    if let Some(ref nts) = newest_ts {
        if cursor.as_deref() != Some(nts.as_str()) {
            db.upsert_social_cursor("instagram", "comments", nts)?;
        }
    }

    debug!(
        new = items.len(),
        total_comments,
        skipped_by_cursor,
        skipped_by_dedup,
        cursor = ?cursor,
        newest_ts = ?newest_ts,
        "instagram poll comments: done"
    );
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
    debug!(cursor = ?cursor, "threads poll replies: starting");

    let timeline = client.get_timeline(20).await?;
    let posts = timeline
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    debug!("threads poll: fetched {} posts", posts.len());

    let mut items = Vec::new();
    let mut newest_ts: Option<String> = cursor.clone();
    let mut total_replies = 0u32;
    let mut skipped_by_cursor = 0u32;
    let mut skipped_by_dedup = 0u32;

    // Collect first-level reply IDs so we can check sub-replies on our replied items
    let mut first_level_ids: Vec<String> = Vec::new();

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

        debug!(post_id, replies = reply_list.len(), "threads poll: post replies");

        for reply in &reply_list {
            let id = match reply.get("id").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => continue,
            };
            let ts = reply
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            total_replies += 1;
            first_level_ids.push(id.clone());
            if let Some(ref last_ts) = cursor {
                if !ts_gt(&ts, last_ts) {
                    skipped_by_cursor += 1;
                    continue;
                }
            }
            if db.get_social_inbox_by_platform_id("threads", &id).ok().flatten().is_some() {
                skipped_by_dedup += 1;
                continue;
            }
            if newest_ts.as_deref().map(|n| ts_gt(&ts, n)).unwrap_or(true) {
                newest_ts = Some(ts.clone());
            }
            let username = reply
                .get("username")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let text = reply
                .get("text")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            debug!(id = %id, ts = %ts, username = ?username, text = ?text, "threads poll: new reply");
            items.push(SocialItem {
                platform: SocialPlatform::Threads,
                platform_id: id,
                event_type: "reply".to_string(),
                author_id: username.clone(),
                author_name: username,
                media_id: Some(post_id.to_string()),
                text,
                metadata: reply.clone(),
            });
        }
    }

    // Check sub-replies on items we have replied to (our reply_id is in social_inbox).
    // This catches "reply to our reply" — the conversation thread we're participating in.
    let our_reply_ids = db.list_replied_platform_ids("threads").unwrap_or_default();
    for reply_id in &our_reply_ids {
        let sub_replies = match client.get_replies(reply_id, None).await {
            Ok(r) => r,
            Err(e) => {
                debug!(reply_id = %reply_id, error = %e, "threads poll: failed to fetch sub-replies");
                continue;
            }
        };
        let sub_list = sub_replies
            .get("data")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        for sub in &sub_list {
            let id = match sub.get("id").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => continue,
            };
            let ts = sub
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(ref last_ts) = cursor {
                if !ts_gt(&ts, last_ts) {
                    continue;
                }
            }
            if newest_ts.as_deref().map(|n| ts_gt(&ts, n)).unwrap_or(true) {
                newest_ts = Some(ts.clone());
            }
            items.push(SocialItem {
                platform: SocialPlatform::Threads,
                platform_id: id,
                event_type: "reply".to_string(),
                author_id: sub
                    .get("username")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                author_name: sub
                    .get("username")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                media_id: Some(reply_id.clone()),
                text: sub
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                metadata: sub.clone(),
            });
        }
    }

    if let Some(ref nts) = newest_ts {
        if cursor.as_deref() != Some(nts.as_str()) {
            db.upsert_social_cursor("threads", "replies", nts)?;
        }
    }

    debug!(
        new = items.len(),
        total_replies,
        skipped_by_cursor,
        skipped_by_dedup,
        cursor = ?cursor,
        newest_ts = ?newest_ts,
        "threads poll replies: done"
    );
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
