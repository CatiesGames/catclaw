use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;

use crate::error::Result;

/// Register sqlite-vec extension as auto-extension. Must be called before Connection::open().
/// Safe to call multiple times (idempotent via std::sync::Once).
static SQLITE_VEC_INIT: std::sync::Once = std::sync::Once::new();

#[allow(clippy::missing_transmute_annotations)]
fn ensure_sqlite_vec() {
    SQLITE_VEC_INIT.call_once(|| {
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
    });
}

type OldSessionRow = (String, String, String, String, String, Option<String>, Option<String>, String, String, String, Option<String>);

/// State database backed by SQLite WAL
pub struct StateDb {
    pub(crate) conn: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub session_key: String,
    pub session_id: String,
    pub agent_id: String,
    pub origin: String,
    pub context_id: String,
    pub parent_session_id: Option<String>,
    pub state: String,
    pub last_activity_at: String,
    pub created_at: String,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ChannelBindingRow {
    pub pattern: String,
    pub agent_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct ScheduledTaskRow {
    pub id: i64,
    pub task_type: String,
    pub agent_id: String,
    pub name: String,
    pub description: Option<String>,
    pub cron_expr: Option<String>,
    pub interval_mins: Option<i64>,
    pub next_run_at: String,
    pub last_run_at: Option<String>,
    pub enabled: bool,
    pub payload: Option<String>,
    /// When true, task reuses the same session across runs (context persists).
    /// Default false: each run starts a fresh session.
    pub keep_context: bool,
    /// When true, diary extraction runs after task session completes.
    /// Default false: system-originated sessions skip diary extraction.
    pub remember: bool,
}

#[allow(dead_code)]
impl SessionRow {
    /// Read the `model` field from the metadata JSON, if present.
    pub fn model(&self) -> Option<String> {
        let meta = self.metadata.as_deref()?;
        let parsed: Value = serde_json::from_str(meta).ok()?;
        parsed.get("model")?.as_str().map(String::from)
    }

    /// Read the platform channel ID from metadata JSON.
    pub fn platform_channel_id(&self) -> Option<String> {
        let meta = self.metadata.as_deref()?;
        let parsed: Value = serde_json::from_str(meta).ok()?;
        parsed.get("channel_id")?.as_str().map(String::from)
    }

    /// Read the platform thread ID from metadata JSON (Slack thread_ts, etc.).
    pub fn platform_thread_id(&self) -> Option<String> {
        let meta = self.metadata.as_deref()?;
        let parsed: Value = serde_json::from_str(meta).ok()?;
        parsed.get("thread_id")?.as_str().map(String::from)
    }

    /// Read the platform sender ID from metadata JSON.
    pub fn platform_sender_id(&self) -> Option<String> {
        let meta = self.metadata.as_deref()?;
        let parsed: Value = serde_json::from_str(meta).ok()?;
        parsed.get("sender_id")?.as_str().map(String::from)
    }

    /// Store the platform channel and sender IDs in metadata JSON.
    pub fn set_platform_ids(&mut self, channel_id: &str, sender_id: &str) {
        let mut obj = self
            .metadata
            .as_deref()
            .and_then(|m| serde_json::from_str::<Value>(m).ok())
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

        if let Some(map) = obj.as_object_mut() {
            map.insert("channel_id".to_string(), Value::String(channel_id.to_string()));
            map.insert("sender_id".to_string(), Value::String(sender_id.to_string()));
        }

        self.metadata = Some(serde_json::to_string(&obj).unwrap());
    }

    /// Set or clear the `model` field in the metadata JSON.
    pub fn set_model_metadata(&mut self, model: Option<&str>) {
        let mut obj = self
            .metadata
            .as_deref()
            .and_then(|m| serde_json::from_str::<Value>(m).ok())
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

        if let Some(map) = obj.as_object_mut() {
            match model {
                Some(m) => {
                    map.insert("model".to_string(), Value::String(m.to_string()));
                }
                None => {
                    map.remove("model");
                }
            }
        }

        self.metadata = Some(serde_json::to_string(&obj).unwrap());
    }
}

#[allow(dead_code)]
impl StateDb {
    /// Open or create the state database
    pub fn open(path: &Path) -> Result<Self> {
        ensure_sqlite_vec();
        let conn = Connection::open(path)?;

        // Enable WAL mode
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let db = StateDb {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    /// Open an in-memory database (for testing)
    pub fn open_memory() -> Result<Self> {
        ensure_sqlite_vec();
        let conn = Connection::open_in_memory()?;
        let db = StateDb {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Check if sessions table exists with old schema (channel_type/channel_id/peer_id)
        let has_old_schema = conn
            .prepare("SELECT channel_type FROM sessions LIMIT 0")
            .is_ok();

        if has_old_schema {
            // Migrate old sessions table to new schema
            // Read all existing sessions
            let mut old_rows: Vec<OldSessionRow> = Vec::new();
            {
                let mut stmt = conn.prepare(
                    "SELECT session_key, session_id, agent_id, channel_type, channel_id, peer_id, parent_session_id, state, last_activity_at, created_at, metadata FROM sessions"
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, Option<String>>(10)?,
                    ))
                })?;
                for row in rows {
                    old_rows.push(row?);
                }
            }

            // Drop old table and create new one
            conn.execute_batch("DROP TABLE sessions")?;
            conn.execute_batch(
                "CREATE TABLE sessions (
                    session_key TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    agent_id TEXT NOT NULL,
                    origin TEXT NOT NULL,
                    context_id TEXT NOT NULL,
                    parent_session_id TEXT,
                    state TEXT NOT NULL DEFAULT 'suspended',
                    last_activity_at TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    metadata TEXT
                )",
            )?;

            // Re-insert with mapped fields: channel_type → origin, channel_id.peer_id → context_id
            for (_session_key, session_id, agent_id, channel_type, channel_id, peer_id, parent_session_id, state, last_activity_at, created_at, metadata) in &old_rows {
                let origin = channel_type.clone();
                let context_id = match peer_id {
                    Some(p) => format!("{}.{}", channel_id, p),
                    None => channel_id.clone(),
                };
                // Rebuild session_key in new format
                let new_key = format!("catclaw:{}:{}:{}", agent_id, origin, context_id);
                conn.execute(
                    "INSERT OR IGNORE INTO sessions (session_key, session_id, agent_id, origin, context_id, parent_session_id, state, last_activity_at, created_at, metadata)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![new_key, session_id, agent_id, origin, context_id, parent_session_id, state, last_activity_at, created_at, metadata],
                )?;
            }

            tracing::info!(count = old_rows.len(), "migrated sessions to new schema (origin/context_id)");
        } else {
            // Fresh install or already migrated — create new schema
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS sessions (
                    session_key TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    agent_id TEXT NOT NULL,
                    origin TEXT NOT NULL,
                    context_id TEXT NOT NULL,
                    parent_session_id TEXT,
                    state TEXT NOT NULL DEFAULT 'suspended',
                    last_activity_at TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    metadata TEXT
                )",
            )?;
        }

        // Other tables (unchanged)
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS scheduled_tasks (
                id INTEGER PRIMARY KEY,
                task_type TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                name TEXT NOT NULL DEFAULT '',
                description TEXT,
                cron_expr TEXT,
                interval_mins INTEGER,
                next_run_at TEXT NOT NULL,
                last_run_at TEXT,
                enabled INTEGER NOT NULL DEFAULT 1,
                payload TEXT
            );

            CREATE TABLE IF NOT EXISTS channel_bindings (
                pattern TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            ",
        )?;

        // Migrations for existing databases
        // Add name/description columns if missing (v0.1.1)
        let _ = conn.execute_batch(
            "ALTER TABLE scheduled_tasks ADD COLUMN name TEXT NOT NULL DEFAULT '';
             ALTER TABLE scheduled_tasks ADD COLUMN description TEXT;",
        );
        // Add keep_context column (default 0 = fresh session each run)
        let _ = conn.execute_batch(
            "ALTER TABLE scheduled_tasks ADD COLUMN keep_context INTEGER NOT NULL DEFAULT 0;",
        );
        // Add remember column (default 0 = no memory extraction)
        let _ = conn.execute_batch(
            "ALTER TABLE scheduled_tasks ADD COLUMN remember INTEGER NOT NULL DEFAULT 0;",
        );

        // Social inbox tables
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS social_inbox (
                id            INTEGER PRIMARY KEY,
                platform      TEXT NOT NULL,
                platform_id   TEXT NOT NULL,
                event_type    TEXT NOT NULL,
                author_id     TEXT,
                author_name   TEXT,
                media_id      TEXT,
                text          TEXT,
                status        TEXT NOT NULL DEFAULT 'pending',
                action        TEXT,
                draft         TEXT,
                reply_id      TEXT,
                session_key   TEXT,
                forward_ref   TEXT,
                metadata      TEXT,
                created_at    TEXT NOT NULL,
                updated_at    TEXT NOT NULL,
                UNIQUE(platform, platform_id)
            );

            CREATE TABLE IF NOT EXISTS social_cursors (
                platform    TEXT NOT NULL,
                feed        TEXT NOT NULL,
                cursor_val  TEXT NOT NULL,
                updated_at  TEXT NOT NULL,
                PRIMARY KEY (platform, feed)
            );

            CREATE TABLE IF NOT EXISTS social_parent_cache (
                platform    TEXT NOT NULL,
                media_id    TEXT NOT NULL,
                text        TEXT,
                permalink   TEXT,
                fetched_at  TEXT NOT NULL,
                PRIMARY KEY (platform, media_id)
            );

            CREATE TABLE IF NOT EXISTS social_drafts (
                id              INTEGER PRIMARY KEY,
                platform        TEXT NOT NULL,
                draft_type      TEXT NOT NULL,
                content         TEXT NOT NULL,
                media_url       TEXT,
                reply_to_id     TEXT,
                original_text   TEXT,
                original_author TEXT,
                status          TEXT NOT NULL DEFAULT 'draft',
                reply_id        TEXT,
                forward_ref     TEXT,
                agent_id        TEXT,
                session_key     TEXT,
                metadata        TEXT,
                created_at      TEXT NOT NULL,
                updated_at      TEXT NOT NULL
            );
            ",
        )?;

        // ── Memory Palace tables ─────────────────────────────────────────────

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS memory_nodes (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                wing        TEXT NOT NULL,
                room        TEXT NOT NULL DEFAULT 'general',
                hall        TEXT NOT NULL DEFAULT 'facts',
                content     TEXT NOT NULL,
                summary     TEXT,
                source      TEXT NOT NULL DEFAULT 'agent',
                importance  INTEGER NOT NULL DEFAULT 5,
                chunk_index INTEGER,
                parent_id   INTEGER,
                created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                metadata    TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_nodes_wing ON memory_nodes(wing);
            CREATE INDEX IF NOT EXISTS idx_nodes_wing_room ON memory_nodes(wing, room);
            CREATE INDEX IF NOT EXISTS idx_nodes_wing_hall ON memory_nodes(wing, hall);
            CREATE INDEX IF NOT EXISTS idx_nodes_importance ON memory_nodes(wing, importance DESC);
            ",
        )?;

        // FTS5 full-text search (content-sync'd with memory_nodes)
        conn.execute_batch(
            "
            CREATE VIRTUAL TABLE IF NOT EXISTS memory_nodes_fts USING fts5(
                content, summary,
                content=memory_nodes, content_rowid=id,
                tokenize='unicode61 remove_diacritics 2'
            );

            CREATE TRIGGER IF NOT EXISTS memory_nodes_ai AFTER INSERT ON memory_nodes BEGIN
                INSERT INTO memory_nodes_fts(rowid, content, summary)
                VALUES (new.id, new.content, new.summary);
            END;
            CREATE TRIGGER IF NOT EXISTS memory_nodes_ad AFTER DELETE ON memory_nodes BEGIN
                INSERT INTO memory_nodes_fts(memory_nodes_fts, rowid, content, summary)
                VALUES ('delete', old.id, old.content, old.summary);
            END;
            CREATE TRIGGER IF NOT EXISTS memory_nodes_au AFTER UPDATE ON memory_nodes BEGIN
                INSERT INTO memory_nodes_fts(memory_nodes_fts, rowid, content, summary)
                VALUES ('delete', old.id, old.content, old.summary);
                INSERT INTO memory_nodes_fts(rowid, content, summary)
                VALUES (new.id, new.content, new.summary);
            END;
            ",
        )?;

        // sqlite-vec vector index (768 dims for multilingual-e5-base, cosine distance)
        conn.execute_batch(
            "
            CREATE VIRTUAL TABLE IF NOT EXISTS vec_memories USING vec0(
                node_id INTEGER PRIMARY KEY,
                embedding float[1024] distance_metric=cosine
            );
            ",
        )?;

        // Knowledge graph tables
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS kg_entities (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                wing        TEXT NOT NULL,
                name        TEXT NOT NULL,
                entity_type TEXT DEFAULT 'unknown',
                created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                UNIQUE(wing, name)
            );
            CREATE INDEX IF NOT EXISTS idx_entities_wing ON kg_entities(wing);

            CREATE TABLE IF NOT EXISTS kg_triples (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                wing            TEXT NOT NULL,
                subject_id      INTEGER NOT NULL REFERENCES kg_entities(id),
                predicate       TEXT NOT NULL,
                object_id       INTEGER NOT NULL REFERENCES kg_entities(id),
                confidence      REAL NOT NULL DEFAULT 1.0,
                source_node_id  INTEGER REFERENCES memory_nodes(id),
                valid_from      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                valid_to        TEXT,
                created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
            );
            CREATE INDEX IF NOT EXISTS idx_triples_wing ON kg_triples(wing);
            CREATE INDEX IF NOT EXISTS idx_triples_subject ON kg_triples(subject_id);
            CREATE INDEX IF NOT EXISTS idx_triples_object ON kg_triples(object_id);
            CREATE INDEX IF NOT EXISTS idx_triples_valid ON kg_triples(valid_to);

            CREATE TABLE IF NOT EXISTS palace_meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            ",
        )?;

        Ok(())
    }

    // --- Sessions ---

    pub fn upsert_session(&self, row: &SessionRow) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (session_key, session_id, agent_id, origin, context_id, parent_session_id, state, last_activity_at, created_at, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(session_key) DO UPDATE SET
                session_id = excluded.session_id,
                state = excluded.state,
                last_activity_at = excluded.last_activity_at,
                metadata = excluded.metadata",
            params![
                row.session_key,
                row.session_id,
                row.agent_id,
                row.origin,
                row.context_id,
                row.parent_session_id,
                row.state,
                row.last_activity_at,
                row.created_at,
                row.metadata,
            ],
        )?;
        Ok(())
    }

    pub fn get_session(&self, session_key: &str) -> Result<Option<SessionRow>> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT session_key, session_id, agent_id, origin, context_id, parent_session_id, state, last_activity_at, created_at, metadata
                 FROM sessions WHERE session_key = ?1",
                params![session_key],
                |row| {
                    Ok(SessionRow {
                        session_key: row.get(0)?,
                        session_id: row.get(1)?,
                        agent_id: row.get(2)?,
                        origin: row.get(3)?,
                        context_id: row.get(4)?,
                        parent_session_id: row.get(5)?,
                        state: row.get(6)?,
                        last_activity_at: row.get(7)?,
                        created_at: row.get(8)?,
                        metadata: row.get(9)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT session_key, session_id, agent_id, origin, context_id, parent_session_id, state, last_activity_at, created_at, metadata
             FROM sessions ORDER BY last_activity_at DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SessionRow {
                    session_key: row.get(0)?,
                    session_id: row.get(1)?,
                    agent_id: row.get(2)?,
                    origin: row.get(3)?,
                    context_id: row.get(4)?,
                    parent_session_id: row.get(5)?,
                    state: row.get(6)?,
                    last_activity_at: row.get(7)?,
                    created_at: row.get(8)?,
                    metadata: row.get(9)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn update_session_state(&self, session_key: &str, state: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions SET state = ?1, last_activity_at = ?2 WHERE session_key = ?3",
            params![state, Utc::now().to_rfc3339(), session_key],
        )?;
        Ok(())
    }

    pub fn suspend_all_active_sessions(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count = conn.execute(
            "UPDATE sessions SET state = 'suspended' WHERE state = 'active'",
            [],
        )?;
        Ok(count)
    }

    pub fn delete_session(&self, session_key: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM sessions WHERE session_key = ?1", params![session_key])?;
        Ok(())
    }

    /// Update the model in a session's metadata JSON.
    pub fn set_session_model(&self, session_key: &str, model: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let current_meta: Option<String> = conn
            .query_row(
                "SELECT metadata FROM sessions WHERE session_key = ?1",
                params![session_key],
                |row| row.get(0),
            )
            .optional()?
            .flatten();

        let mut obj = current_meta
            .as_deref()
            .and_then(|m| serde_json::from_str::<Value>(m).ok())
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

        if let Some(map) = obj.as_object_mut() {
            match model {
                Some(m) => {
                    map.insert("model".to_string(), Value::String(m.to_string()));
                }
                None => {
                    map.remove("model");
                }
            }
        }

        let new_meta = serde_json::to_string(&obj).unwrap();
        conn.execute(
            "UPDATE sessions SET metadata = ?1 WHERE session_key = ?2",
            params![new_meta, session_key],
        )?;
        Ok(())
    }

    // --- Channel Bindings ---

    pub fn upsert_binding(&self, pattern: &str, agent_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO channel_bindings (pattern, agent_id, created_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(pattern) DO UPDATE SET agent_id = excluded.agent_id",
            params![pattern, agent_id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn list_bindings(&self) -> Result<Vec<ChannelBindingRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT pattern, agent_id, created_at FROM channel_bindings ORDER BY pattern",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ChannelBindingRow {
                    pattern: row.get(0)?,
                    agent_id: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_binding(&self, pattern: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM channel_bindings WHERE pattern = ?1",
            params![pattern],
        )?;
        Ok(())
    }

    // --- Scheduled Tasks ---

    pub fn list_scheduled_tasks(&self) -> Result<Vec<ScheduledTaskRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, task_type, agent_id, name, description, cron_expr, interval_mins, next_run_at, last_run_at, enabled, payload, keep_context, remember
             FROM scheduled_tasks ORDER BY next_run_at",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ScheduledTaskRow {
                    id: row.get(0)?,
                    task_type: row.get(1)?,
                    agent_id: row.get(2)?,
                    name: row.get(3)?,
                    description: row.get(4)?,
                    cron_expr: row.get(5)?,
                    interval_mins: row.get(6)?,
                    next_run_at: row.get(7)?,
                    last_run_at: row.get(8)?,
                    enabled: row.get::<_, i32>(9)? != 0,
                    payload: row.get(10)?,
                    keep_context: row.get::<_, i32>(11).unwrap_or(0) != 0,
                    remember: row.get::<_, i32>(12).unwrap_or(0) != 0,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_due_tasks(&self, now: &str) -> Result<Vec<ScheduledTaskRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, task_type, agent_id, name, description, cron_expr, interval_mins, next_run_at, last_run_at, enabled, payload, keep_context, remember
             FROM scheduled_tasks WHERE enabled = 1 AND next_run_at <= ?1 ORDER BY next_run_at",
        )?;
        let rows = stmt
            .query_map(params![now], |row| {
                Ok(ScheduledTaskRow {
                    id: row.get(0)?,
                    task_type: row.get(1)?,
                    agent_id: row.get(2)?,
                    name: row.get(3)?,
                    description: row.get(4)?,
                    cron_expr: row.get(5)?,
                    interval_mins: row.get(6)?,
                    next_run_at: row.get(7)?,
                    last_run_at: row.get(8)?,
                    enabled: row.get::<_, i32>(9)? != 0,
                    payload: row.get(10)?,
                    keep_context: row.get::<_, i32>(11).unwrap_or(0) != 0,
                    remember: row.get::<_, i32>(12).unwrap_or(0) != 0,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn insert_task(&self, task: &ScheduledTaskRow) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO scheduled_tasks (task_type, agent_id, name, description, cron_expr, interval_mins, next_run_at, last_run_at, enabled, payload, keep_context, remember)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                task.task_type,
                task.agent_id,
                task.name,
                task.description,
                task.cron_expr,
                task.interval_mins,
                task.next_run_at,
                task.last_run_at,
                task.enabled as i32,
                task.payload,
                task.keep_context as i32,
                task.remember as i32,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_task_schedule(&self, id: i64, next_run_at: &str, last_run_at: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE scheduled_tasks SET next_run_at = ?1, last_run_at = ?2 WHERE id = ?3",
            params![next_run_at, last_run_at, id],
        )?;
        Ok(())
    }

    pub fn disable_task(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE scheduled_tasks SET enabled = 0 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn enable_task(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE scheduled_tasks SET enabled = 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn delete_task(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM scheduled_tasks WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    /// Find a task ID by name. Returns the first match.
    pub fn find_task_id_by_name(&self, name: &str) -> Result<Option<i64>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id FROM scheduled_tasks WHERE name = ?1 LIMIT 1")?;
        let id = stmt.query_row(params![name], |row| row.get(0)).optional()?;
        Ok(id)
    }

    pub fn get_task(&self, id: i64) -> Result<Option<ScheduledTaskRow>> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT id, task_type, agent_id, name, description, cron_expr, interval_mins, next_run_at, last_run_at, enabled, payload, keep_context, remember
                 FROM scheduled_tasks WHERE id = ?1",
                params![id],
                |row| {
                    Ok(ScheduledTaskRow {
                        id: row.get(0)?,
                        task_type: row.get(1)?,
                        agent_id: row.get(2)?,
                        name: row.get(3)?,
                        description: row.get(4)?,
                        cron_expr: row.get(5)?,
                        interval_mins: row.get(6)?,
                        next_run_at: row.get(7)?,
                        last_run_at: row.get(8)?,
                        enabled: row.get::<_, i32>(9)? != 0,
                        payload: row.get(10)?,
                        keep_context: row.get::<_, i32>(11).unwrap_or(0) != 0,
                        remember: row.get::<_, i32>(12).unwrap_or(0) != 0,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    // --- Social Inbox ---

    /// Insert a new social inbox item. Returns false if already exists (dedup).
    pub fn insert_social_inbox(&self, row: &SocialInboxRow) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "INSERT OR IGNORE INTO social_inbox
             (platform, platform_id, event_type, author_id, author_name, media_id, text,
              status, action, draft, reply_id, session_key, forward_ref, metadata, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            params![
                row.platform, row.platform_id, row.event_type,
                row.author_id, row.author_name, row.media_id, row.text,
                row.status, row.action, row.draft, row.reply_id,
                row.session_key, row.forward_ref, row.metadata,
                row.created_at, row.updated_at,
            ],
        )?;
        Ok(affected > 0)
    }

    pub fn get_social_inbox_by_platform_id(&self, platform: &str, platform_id: &str) -> Result<Option<SocialInboxRow>> {
        let conn = self.conn.lock().unwrap();
        let row = conn.query_row(
            "SELECT id,platform,platform_id,event_type,author_id,author_name,media_id,text,
                    status,action,draft,reply_id,session_key,forward_ref,metadata,created_at,updated_at
             FROM social_inbox WHERE platform=?1 AND platform_id=?2",
            params![platform, platform_id],
            social_inbox_row_mapper,
        ).optional()?;
        Ok(row)
    }

    pub fn get_social_inbox(&self, id: i64) -> Result<Option<SocialInboxRow>> {
        let conn = self.conn.lock().unwrap();
        let row = conn.query_row(
            "SELECT id,platform,platform_id,event_type,author_id,author_name,media_id,text,
                    status,action,draft,reply_id,session_key,forward_ref,metadata,created_at,updated_at
             FROM social_inbox WHERE id=?1",
            params![id],
            social_inbox_row_mapper,
        ).optional()?;
        Ok(row)
    }

    pub fn list_social_inbox(&self, platform_filter: Option<&str>, status_filter: Option<&str>, limit: i64) -> Result<Vec<SocialInboxRow>> {
        const COLS: &str = "id,platform,platform_id,event_type,author_id,author_name,media_id,text,status,action,draft,reply_id,session_key,forward_ref,metadata,created_at,updated_at";
        let conn = self.conn.lock().unwrap();
        let rows: Vec<SocialInboxRow> = match (platform_filter, status_filter) {
            (Some(p), Some(s)) => {
                let sql = format!("SELECT {COLS} FROM social_inbox WHERE platform=?1 AND status=?2 ORDER BY created_at DESC LIMIT ?3");
                conn.prepare(&sql)?.query_map(params![p, s, limit], social_inbox_row_mapper)?
                    .filter_map(|r| r.ok()).collect()
            }
            (Some(p), None) => {
                let sql = format!("SELECT {COLS} FROM social_inbox WHERE platform=?1 ORDER BY created_at DESC LIMIT ?2");
                conn.prepare(&sql)?.query_map(params![p, limit], social_inbox_row_mapper)?
                    .filter_map(|r| r.ok()).collect()
            }
            (None, Some(s)) => {
                let sql = format!("SELECT {COLS} FROM social_inbox WHERE status=?1 ORDER BY created_at DESC LIMIT ?2");
                conn.prepare(&sql)?.query_map(params![s, limit], social_inbox_row_mapper)?
                    .filter_map(|r| r.ok()).collect()
            }
            (None, None) => {
                let sql = format!("SELECT {COLS} FROM social_inbox ORDER BY created_at DESC LIMIT ?1");
                conn.prepare(&sql)?.query_map(params![limit], social_inbox_row_mapper)?
                    .filter_map(|r| r.ok()).collect()
            }
        };
        Ok(rows)
    }

    pub fn update_social_inbox_status(&self, id: i64, status: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE social_inbox SET status=?1, updated_at=?2 WHERE id=?3",
            params![status, now, id],
        )?;
        Ok(())
    }

    /// Reset an inbox item for reprocessing: clear draft/reply/forward/session and set status=pending.
    pub fn reset_social_inbox_for_reprocess(&self, id: i64) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE social_inbox SET status='pending', action=NULL, draft=NULL, reply_id=NULL, \
             session_key=NULL, forward_ref=NULL, updated_at=?1 WHERE id=?2",
            params![now, id],
        )?;
        Ok(())
    }

    pub fn update_social_inbox_draft(&self, id: i64, draft: &str, status: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE social_inbox SET draft=?1, status=?2, updated_at=?3 WHERE id=?4",
            params![draft, status, now, id],
        )?;
        Ok(())
    }

    pub fn update_social_inbox_forward_ref(&self, id: i64, forward_ref: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE social_inbox SET forward_ref=?1, status='forwarded', updated_at=?2 WHERE id=?3",
            params![forward_ref, now, id],
        )?;
        Ok(())
    }

    pub fn update_social_inbox_sent(&self, id: i64, reply_id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE social_inbox SET reply_id=?1, status='sent', updated_at=?2 WHERE id=?3",
            params![reply_id, now, id],
        )?;
        Ok(())
    }

    pub fn update_social_inbox_session(&self, id: i64, session_key: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE social_inbox SET session_key=?1, status='auto_replying', updated_at=?2 WHERE id=?3",
            params![session_key, now, id],
        )?;
        Ok(())
    }

    /// Update status by (platform, platform_id) — no prior lookup required.
    pub fn set_social_inbox_status_by_platform_id(
        &self,
        platform: &str,
        platform_id: &str,
        status: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE social_inbox SET status=?1, updated_at=?2 WHERE platform=?3 AND platform_id=?4",
            params![status, now, platform, platform_id],
        )?;
        Ok(())
    }

    /// Mark as sent (reply_id + status=sent) by (platform, platform_id) — no prior lookup.
    pub fn set_social_inbox_sent_by_platform_id(
        &self,
        platform: &str,
        platform_id: &str,
        reply_id: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE social_inbox SET reply_id=?1, status='sent', updated_at=?2 WHERE platform=?3 AND platform_id=?4",
            params![reply_id, now, platform, platform_id],
        )?;
        Ok(())
    }

    // --- Social Drafts ---

    pub fn insert_social_draft(&self, row: &SocialDraftRow) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO social_drafts
             (platform, draft_type, content, media_url, reply_to_id, original_text, original_author,
              status, reply_id, forward_ref, agent_id, session_key, metadata, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            params![
                row.platform, row.draft_type, row.content, encode_media_urls(&row.media_urls),
                row.reply_to_id, row.original_text, row.original_author,
                row.status, row.reply_id, row.forward_ref, row.agent_id,
                row.session_key, row.metadata, row.created_at, row.updated_at,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_social_draft(&self, id: i64) -> Result<Option<SocialDraftRow>> {
        let conn = self.conn.lock().unwrap();
        let row = conn.query_row(
            "SELECT id,platform,draft_type,content,media_url,reply_to_id,original_text,original_author,
                    status,reply_id,forward_ref,agent_id,session_key,metadata,created_at,updated_at
             FROM social_drafts WHERE id=?1",
            params![id],
            social_draft_row_mapper,
        ).optional()?;
        Ok(row)
    }

    pub fn list_social_drafts(
        &self,
        platform_filter: Option<&str>,
        status_filter: Option<&str>,
        limit: i64,
    ) -> Result<Vec<SocialDraftRow>> {
        const COLS: &str = "id,platform,draft_type,content,media_url,reply_to_id,original_text,\
                            original_author,status,reply_id,forward_ref,agent_id,session_key,\
                            metadata,created_at,updated_at";
        let conn = self.conn.lock().unwrap();
        let rows: Vec<SocialDraftRow> = match (platform_filter, status_filter) {
            (Some(p), Some(s)) => {
                let sql = format!("SELECT {COLS} FROM social_drafts WHERE platform=?1 AND status=?2 ORDER BY created_at DESC LIMIT ?3");
                conn.prepare(&sql)?.query_map(params![p, s, limit], social_draft_row_mapper)?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            }
            (Some(p), None) => {
                let sql = format!("SELECT {COLS} FROM social_drafts WHERE platform=?1 ORDER BY created_at DESC LIMIT ?2");
                conn.prepare(&sql)?.query_map(params![p, limit], social_draft_row_mapper)?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            }
            (None, Some(s)) => {
                let sql = format!("SELECT {COLS} FROM social_drafts WHERE status=?1 ORDER BY created_at DESC LIMIT ?2");
                conn.prepare(&sql)?.query_map(params![s, limit], social_draft_row_mapper)?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            }
            (None, None) => {
                let sql = format!("SELECT {COLS} FROM social_drafts ORDER BY created_at DESC LIMIT ?1");
                conn.prepare(&sql)?.query_map(params![limit], social_draft_row_mapper)?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            }
        };
        Ok(rows)
    }

    pub fn update_social_draft_status(&self, id: i64, status: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE social_drafts SET status=?1, updated_at=?2 WHERE id=?3",
            params![status, now, id],
        )?;
        Ok(())
    }

    pub fn update_social_draft_original(&self, id: i64, author: &str, text: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE social_drafts SET original_author=?1, original_text=?2, updated_at=?3 WHERE id=?4",
            params![author, text, now, id],
        )?;
        Ok(())
    }

    pub fn delete_social_draft(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM social_drafts WHERE id=?1", params![id])?;
        Ok(())
    }

    pub fn update_social_draft_sent(&self, id: i64, reply_id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE social_drafts SET reply_id=?1, status='sent', updated_at=?2 WHERE id=?3",
            params![reply_id, now, id],
        )?;
        Ok(())
    }

    pub fn update_social_draft_forward_ref(&self, id: i64, forward_ref: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE social_drafts SET forward_ref=?1, updated_at=?2 WHERE id=?3",
            params![forward_ref, now, id],
        )?;
        Ok(())
    }

    /// Refresh the content + media of an existing draft. Used when a reused draft
    /// (from `find_latest_draft_for_tool`) needs to pick up new agent output on retry.
    pub fn update_social_draft_content(&self, id: i64, content: &str, media_urls: &[String]) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let encoded = encode_media_urls(media_urls);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE social_drafts SET content=?1, media_url=?2, updated_at=?3 WHERE id=?4",
            params![content, encoded, now, id],
        )?;
        Ok(())
    }

    /// Find the latest staged draft matching platform + draft_type (+ optional reply_to_id).
    /// Used after a publish tool call to associate the draft.
    pub fn find_latest_draft_for_tool(
        &self,
        platform: &str,
        draft_type: &str,
        reply_to_id: Option<&str>,
    ) -> Result<Option<SocialDraftRow>> {
        const COLS: &str = "id,platform,draft_type,content,media_url,reply_to_id,original_text,\
                            original_author,status,reply_id,forward_ref,agent_id,session_key,\
                            metadata,created_at,updated_at";
        // Reuse any non-terminal draft for the same target so retries / re-approvals
        // do not leave zombie rows. Terminal statuses (sent, ignored) are excluded.
        let conn = self.conn.lock().unwrap();
        let row = if let Some(rid) = reply_to_id {
            conn.query_row(
                &format!("SELECT {COLS} FROM social_drafts WHERE platform=?1 AND draft_type=?2 AND reply_to_id=?3 AND status IN ('draft','awaiting_approval','failed') ORDER BY created_at DESC LIMIT 1"),
                params![platform, draft_type, rid],
                social_draft_row_mapper,
            ).optional()?
        } else {
            conn.query_row(
                &format!("SELECT {COLS} FROM social_drafts WHERE platform=?1 AND draft_type=?2 AND status IN ('draft','awaiting_approval','failed') ORDER BY created_at DESC LIMIT 1"),
                params![platform, draft_type],
                social_draft_row_mapper,
            ).optional()?
        };
        Ok(row)
    }

    /// List platform_id values of inbox items we have replied to (status='sent' with reply_id).
    /// Used by poller to know which replies to check for sub-replies.
    pub fn list_replied_platform_ids(&self, platform: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT reply_id FROM social_inbox WHERE platform=?1 AND status='sent' AND reply_id IS NOT NULL",
        )?;
        let ids = stmt.query_map(params![platform], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }

    // --- Social Cursors ---

    pub fn get_social_cursor(&self, platform: &str, feed: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let val = conn.query_row(
            "SELECT cursor_val FROM social_cursors WHERE platform=?1 AND feed=?2",
            params![platform, feed],
            |row| row.get::<_, String>(0),
        ).optional()?;
        Ok(val)
    }

    pub fn upsert_social_cursor(&self, platform: &str, feed: &str, cursor_val: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO social_cursors (platform, feed, cursor_val, updated_at)
             VALUES (?1,?2,?3,?4)
             ON CONFLICT(platform,feed) DO UPDATE SET cursor_val=excluded.cursor_val, updated_at=excluded.updated_at",
            params![platform, feed, cursor_val, now],
        )?;
        Ok(())
    }

    // --- Social Parent Cache ---

    /// Get cached parent post text and permalink. Returns None if not cached.
    pub fn get_parent_cache(&self, platform: &str, media_id: &str) -> Result<Option<(String, Option<String>)>> {
        let conn = self.conn.lock().unwrap();
        let row = conn.query_row(
            "SELECT text, permalink FROM social_parent_cache WHERE platform=?1 AND media_id=?2",
            params![platform, media_id],
            |row| Ok((
                row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                row.get::<_, Option<String>>(1)?,
            )),
        ).optional()?;
        Ok(row)
    }

    /// Insert or update cached parent post text.
    pub fn upsert_parent_cache(&self, platform: &str, media_id: &str, text: &str, permalink: Option<&str>) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO social_parent_cache (platform, media_id, text, permalink, fetched_at)
             VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(platform,media_id) DO UPDATE SET text=excluded.text, permalink=excluded.permalink, fetched_at=excluded.fetched_at",
            params![platform, media_id, text, permalink, now],
        )?;
        Ok(())
    }
}

// --- Social Inbox Row ---

#[derive(Debug, Clone, serde::Serialize)]
#[allow(dead_code)]
pub struct SocialInboxRow {
    pub id: i64,
    pub platform: String,
    pub platform_id: String,
    pub event_type: String,
    pub author_id: Option<String>,
    pub author_name: Option<String>,
    pub media_id: Option<String>,
    pub text: Option<String>,
    /// Status: pending | forwarded | auto_replying | draft_ready | approved | sent | ignored | failed
    pub status: String,
    pub action: Option<String>,
    pub draft: Option<String>,
    pub reply_id: Option<String>,
    pub session_key: Option<String>,
    pub forward_ref: Option<String>,
    pub metadata: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl SocialInboxRow {
    pub fn new(platform: &str, platform_id: &str, event_type: &str) -> Self {
        let now = Utc::now().to_rfc3339();
        SocialInboxRow {
            id: 0,
            platform: platform.to_string(),
            platform_id: platform_id.to_string(),
            event_type: event_type.to_string(),
            author_id: None,
            author_name: None,
            media_id: None,
            text: None,
            status: "pending".to_string(),
            action: None,
            draft: None,
            reply_id: None,
            session_key: None,
            forward_ref: None,
            metadata: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

fn social_inbox_row_mapper(row: &rusqlite::Row) -> rusqlite::Result<SocialInboxRow> {
    Ok(SocialInboxRow {
        id: row.get(0)?,
        platform: row.get(1)?,
        platform_id: row.get(2)?,
        event_type: row.get(3)?,
        author_id: row.get(4)?,
        author_name: row.get(5)?,
        media_id: row.get(6)?,
        text: row.get(7)?,
        status: row.get(8)?,
        action: row.get(9)?,
        draft: row.get(10)?,
        reply_id: row.get(11)?,
        session_key: row.get(12)?,
        forward_ref: row.get(13)?,
        metadata: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
    })
}

// --- Social Draft Row ---

#[derive(Debug, Clone, serde::Serialize)]
#[allow(dead_code)]
pub struct SocialDraftRow {
    pub id: i64,
    pub platform: String,
    /// "reply" | "post" | "dm"
    pub draft_type: String,
    pub content: String,
    /// Media URLs for the draft. Single image = 1 entry, carousel = 2+ entries.
    /// Stored in DB as: NULL (empty), plain string (1 URL), JSON array (2+ URLs).
    pub media_urls: Vec<String>,
    pub reply_to_id: Option<String>,
    pub original_text: Option<String>,
    pub original_author: Option<String>,
    /// "draft" | "awaiting_approval" | "sent" | "failed" | "ignored"
    pub status: String,
    pub reply_id: Option<String>,
    pub forward_ref: Option<String>,
    pub agent_id: Option<String>,
    pub session_key: Option<String>,
    pub metadata: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl SocialDraftRow {
    pub fn new(platform: &str, draft_type: &str, content: &str) -> Self {
        let now = Utc::now().to_rfc3339();
        SocialDraftRow {
            id: 0,
            platform: platform.to_string(),
            draft_type: draft_type.to_string(),
            content: content.to_string(),
            media_urls: vec![],
            reply_to_id: None,
            original_text: None,
            original_author: None,
            status: "draft".to_string(),
            reply_id: None,
            forward_ref: None,
            agent_id: None,
            session_key: None,
            metadata: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

/// Encode media URLs for DB storage: 0 → NULL, 1 → plain string, 2+ → JSON array.
fn encode_media_urls(urls: &[String]) -> Option<String> {
    match urls.len() {
        0 => None,
        1 => Some(urls[0].clone()),
        _ => Some(serde_json::to_string(urls).unwrap_or_default()),
    }
}

/// Decode media URLs from DB: NULL → empty, JSON array → vec, plain string → vec![s].
fn decode_media_urls(raw: Option<String>) -> Vec<String> {
    match raw {
        None => vec![],
        Some(s) if s.starts_with('[') => {
            serde_json::from_str(&s).unwrap_or_else(|_| vec![s])
        }
        Some(s) => vec![s],
    }
}

fn social_draft_row_mapper(row: &rusqlite::Row) -> rusqlite::Result<SocialDraftRow> {
    Ok(SocialDraftRow {
        id: row.get(0)?,
        platform: row.get(1)?,
        draft_type: row.get(2)?,
        content: row.get(3)?,
        media_urls: decode_media_urls(row.get(4)?),
        reply_to_id: row.get(5)?,
        original_text: row.get(6)?,
        original_author: row.get(7)?,
        status: row.get(8)?,
        reply_id: row.get(9)?,
        forward_ref: row.get(10)?,
        agent_id: row.get(11)?,
        session_key: row.get(12)?,
        metadata: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}
