use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;

use crate::error::Result;

type OldSessionRow = (String, String, String, String, String, Option<String>, Option<String>, String, String, String, Option<String>);

/// State database backed by SQLite WAL
pub struct StateDb {
    conn: Mutex<Connection>,
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
            "SELECT id, task_type, agent_id, name, description, cron_expr, interval_mins, next_run_at, last_run_at, enabled, payload
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
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_due_tasks(&self, now: &str) -> Result<Vec<ScheduledTaskRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, task_type, agent_id, name, description, cron_expr, interval_mins, next_run_at, last_run_at, enabled, payload
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
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn insert_task(&self, task: &ScheduledTaskRow) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO scheduled_tasks (task_type, agent_id, name, description, cron_expr, interval_mins, next_run_at, last_run_at, enabled, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
                "SELECT id, task_type, agent_id, name, description, cron_expr, interval_mins, next_run_at, last_run_at, enabled, payload
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
                    })
                },
            )
            .optional()?;
        Ok(row)
    }
}
