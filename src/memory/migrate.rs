use std::path::Path;
use std::sync::Arc;

use tracing::{info, warn};

use crate::config::AgentConfig;
use crate::error::Result;
use crate::memory::WriteRequest;
use crate::state::StateDb;

const MIGRATION_KEY: &str = "migration_v1";

/// Migrated node info for background post-processing.
struct MigratedNode {
    wing: String,
    node_id: i64,
    content: String,
}

/// Run one-time migration from markdown diary + MEMORY.md to palace DB.
/// Idempotent: checks palace_meta for migration_v1 key before running.
/// After importing, spawns a background task for Haiku analysis + embedding.
pub fn run_migration(
    state_db: &Arc<StateDb>,
    agents: &[AgentConfig],
    workspace: &Path,
    embedder: &Arc<tokio::sync::OnceCell<crate::memory::embed::Embedder>>,
) -> Result<()> {
    if state_db.palace_meta_get(MIGRATION_KEY)?.is_some() {
        return Ok(()); // Already migrated
    }

    info!("memory palace: starting migration from markdown files");

    let mut total_imported = 0usize;
    let mut migrated_nodes: Vec<MigratedNode> = Vec::new();

    for agent_config in agents {
        let agent_workspace = workspace.join("agents").join(&agent_config.id);
        if !agent_workspace.exists() {
            continue;
        }

        let wing = &agent_config.id;

        // 1. Migrate diary files (memory/YYYY-MM-DD.md)
        let memory_dir = agent_workspace.join("memory");
        if memory_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&memory_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    // Match YYYY-MM-DD.md pattern
                    if name_str.len() == 13 && name_str.ends_with(".md") && !name_str.starts_with('.') {
                        let path = entry.path();
                        match import_diary_file(state_db, wing, &path) {
                            Ok(nodes) => {
                                total_imported += nodes.len();
                                if !nodes.is_empty() {
                                    info!(agent = %wing, file = %name_str, entries = nodes.len(), "migrated diary file");
                                }
                                migrated_nodes.extend(nodes);
                            }
                            Err(e) => {
                                warn!(agent = %wing, file = %name_str, error = %e, "failed to migrate diary file");
                            }
                        }
                    }
                }
            }
        }

        // 2. Migrate MEMORY.md
        let memory_md = agent_workspace.join("MEMORY.md");
        if memory_md.exists() {
            match import_memory_md(state_db, wing, &memory_md) {
                Ok(nodes) => {
                    total_imported += nodes.len();
                    if !nodes.is_empty() {
                        info!(agent = %wing, entries = nodes.len(), "migrated MEMORY.md");
                    }
                    migrated_nodes.extend(nodes);
                }
                Err(e) => {
                    warn!(agent = %wing, error = %e, "failed to migrate MEMORY.md");
                }
            }
        }
    }

    // NOTE: Migration is marked complete before background analysis finishes.
    // If gateway restarts during analysis, nodes will lack embeddings/summaries/KG.
    // They remain searchable via FTS5 but not via vector search.
    // This is an acceptable degraded state — the alternative (blocking startup
    // until all nodes are analyzed) could take minutes for large migrations.
    let now = chrono::Utc::now().to_rfc3339();
    state_db.palace_meta_set(MIGRATION_KEY, &now)?;

    info!(total_imported, "memory palace: migration complete, spawning background analysis");

    // Spawn background task for Haiku analysis + embedding on migrated nodes
    if !migrated_nodes.is_empty() {
        let db = state_db.clone();
        let emb = embedder.clone();
        let total = migrated_nodes.len();
        tokio::spawn(async move {
            for (i, node) in migrated_nodes.into_iter().enumerate() {
                if let Err(e) = crate::memory::analyze::analyze_diary(
                    &db,
                    Some(&emb),
                    &node.wing,
                    node.node_id,
                    &node.content,
                )
                .await
                {
                    warn!(node_id = node.node_id, error = %e, "migration: background analysis failed");
                }
                if (i + 1) % 10 == 0 || i + 1 == total {
                    info!(progress = i + 1, total, "migration: background analysis progress");
                }
            }
            info!(total, "migration: background analysis complete");
        });
    }

    Ok(())
}

/// Import a daily diary file (memory/YYYY-MM-DD.md).
/// Each `### channel — time` section becomes one memory_node.
/// Diary format:
/// ```
/// ---
///
/// ### channel context_id — HH:MM
///
/// (diary content here, may span many lines)
///
/// ---
///
/// ### another channel — HH:MM
/// ...
/// ```
fn import_diary_file(state_db: &StateDb, wing: &str, path: &Path) -> Result<Vec<MigratedNode>> {
    let content = std::fs::read_to_string(path)?;
    if content.trim().is_empty() {
        return Ok(vec![]);
    }

    let mut nodes = Vec::new();

    // Split on "### " headers — each section starts with a ### line
    let mut current_room = "general".to_string();
    let mut current_body = String::new();

    for line in content.lines() {
        let trimmed_line = line.trim();

        // Skip standalone "---" delimiters
        if trimmed_line == "---" {
            continue;
        }

        // New section starts with ###
        if trimmed_line.starts_with("### ") {
            // Save previous section if any
            let body = current_body.trim().to_string();
            if !body.is_empty() {
                let req = WriteRequest {
                    wing: wing.to_string(),
                    room: current_room.clone(),
                    hall: "events".to_string(),
                    content: body.clone(),
                    summary: None,
                    source: "migration".to_string(),
                    importance: Some(5),
                };
                let node_id = state_db.memory_write(&req)?;
                nodes.push(MigratedNode {
                    wing: wing.to_string(),
                    node_id,
                    content: body,
                });
            }

            // Parse new header: "### discord 123.chat — 13:04"
            current_room = trimmed_line
                .trim_start_matches('#')
                .split_whitespace()
                .next()
                .unwrap_or("general")
                .to_string();
            current_body.clear();
        } else {
            // Accumulate body lines
            current_body.push_str(line);
            current_body.push('\n');
        }
    }

    // Save last section
    let body = current_body.trim().to_string();
    if !body.is_empty() {
        let req = WriteRequest {
            wing: wing.to_string(),
            room: current_room,
            hall: "events".to_string(),
            content: body.clone(),
            summary: None,
            source: "migration".to_string(),
            importance: Some(5),
        };
        let node_id = state_db.memory_write(&req)?;
        nodes.push(MigratedNode {
            wing: wing.to_string(),
            node_id,
            content: body,
        });
    }

    Ok(nodes)
}

/// Import MEMORY.md — store as a single node, let Haiku analyze and extract facts.
/// MEMORY.md format varies (user-written), so we don't try to parse it ourselves.
fn import_memory_md(state_db: &StateDb, wing: &str, path: &Path) -> Result<Vec<MigratedNode>> {
    let content = std::fs::read_to_string(path)?;
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() {
        return Ok(vec![]);
    }

    // Store as one node — Haiku's analyze_diary will extract summary, room, facts, and KG
    let req = WriteRequest {
        wing: wing.to_string(),
        room: "general".to_string(),
        hall: "facts".to_string(),
        content: trimmed.clone(),
        summary: None,
        source: "migration".to_string(),
        importance: Some(7), // Survived distillation = important
    };

    let node_id = state_db.memory_write(&req)?;
    Ok(vec![MigratedNode {
        wing: wing.to_string(),
        node_id,
        content: trimmed,
    }])
}

/// Backfill missing data at startup: Haiku analysis (summary/room/facts/KG) + embeddings.
/// Runs as a background task — skips quickly if nothing to do.
pub async fn backfill_all(
    state_db: &crate::state::StateDb,
    embedder: &Arc<tokio::sync::OnceCell<crate::memory::embed::Embedder>>,
) {
    // 1. Backfill Haiku analysis — nodes without summary (never analyzed)
    backfill_analysis(state_db, embedder).await;

    // 2. Backfill embeddings — nodes not in vec_memories
    backfill_embeddings(state_db, embedder).await;
}

/// Find memory nodes that were never analyzed by Haiku (no summary) and run analysis.
async fn backfill_analysis(
    state_db: &crate::state::StateDb,
    embedder: &Arc<tokio::sync::OnceCell<crate::memory::embed::Embedder>>,
) {
    // Find nodes without summary (chunk nodes excluded — only analyze primary nodes)
    let missing: Vec<(i64, String, String)> = match (|| -> std::result::Result<Vec<_>, rusqlite::Error> {
        let conn = state_db.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, wing, content FROM memory_nodes
             WHERE summary IS NULL AND chunk_index IS NULL
               AND source != 'extraction'
             LIMIT 500",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    })() {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "backfill_analysis: query failed");
            return;
        }
    };

    if missing.is_empty() {
        return;
    }

    info!(count = missing.len(), "backfill_analysis: analyzing nodes missing summary");

    let mut done = 0usize;
    let mut consecutive_failures = 0usize;
    for (id, wing, content) in &missing {
        match crate::memory::analyze::analyze_diary(state_db, Some(embedder), wing, *id, content).await {
            Ok(()) => {
                done += 1;
                consecutive_failures = 0;
            }
            Err(e) => {
                let err_str = e.to_string();
                warn!(node_id = id, error = %e, "backfill_analysis: failed");
                consecutive_failures += 1;

                // If CLI exited with status 1 (likely rate limit or transient error),
                // don't mark the node — let it retry next startup.
                // Only mark for permanent failures (parse errors, etc.)
                if !err_str.contains("exit status") {
                    let _ = state_db.memory_update_analysis(*id, "", "general", None);
                }

                if consecutive_failures >= 5 {
                    warn!(done, total = missing.len(), "backfill_analysis: pausing after 5 consecutive failures (will retry next startup)");
                    break;
                }
            }
        }
        if done > 0 && done.is_multiple_of(10) {
            info!(done, total = missing.len(), "backfill_analysis: progress");
        }
        // Delay between calls to avoid rate limiting
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }

    info!(done, total = missing.len(), "backfill_analysis: complete");
}

/// Find memory nodes missing embeddings and generate them.
async fn backfill_embeddings(
    state_db: &crate::state::StateDb,
    embedder: &Arc<tokio::sync::OnceCell<crate::memory::embed::Embedder>>,
) {
    let emb = match embedder.get() {
        Some(e) => e,
        None => return,
    };

    let missing: Vec<(i64, String)> = match (|| -> std::result::Result<Vec<(i64, String)>, rusqlite::Error> {
        let conn = state_db.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT m.id, m.content FROM memory_nodes m
             WHERE m.id NOT IN (SELECT node_id FROM vec_memories)
             LIMIT 1000",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    })() {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "backfill_embeddings: query failed");
            return;
        }
    };

    if missing.is_empty() {
        return;
    }

    info!(count = missing.len(), "backfill_embeddings: generating missing embeddings");

    let mut done = 0usize;
    for (id, content) in &missing {
        match emb.embed_one(content).await {
            Ok(vec) => {
                let _ = state_db.memory_insert_embedding(*id, &vec);
                done += 1;
            }
            Err(e) => {
                warn!(node_id = id, error = %e, "backfill_embeddings: embedding failed");
            }
        }
        if done > 0 && done.is_multiple_of(50) {
            info!(done, total = missing.len(), "backfill_embeddings: progress");
        }
    }

    info!(done, total = missing.len(), "backfill_embeddings: complete");
}
