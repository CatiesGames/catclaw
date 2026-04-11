use rusqlite::{params, OptionalExtension};

use crate::error::Result;
use crate::memory::{MemoryNode, PalaceStatus, RoomInfo, TunnelInfo, WingInfo, WriteRequest};
use crate::state::StateDb;

impl StateDb {
    /// Write a memory node to the palace. Returns the inserted row ID.
    /// Deduplicates: skips if an identical or near-identical memory already exists
    /// in the same wing+hall. Returns the existing ID if duplicate found.
    pub fn memory_write(&self, req: &WriteRequest) -> Result<i64> {
        let conn = self.conn.lock().unwrap();

        // Dedup check: exact content match in same wing+hall+room
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM memory_nodes WHERE wing = ?1 AND hall = ?2 AND room = ?3 AND content = ?4 LIMIT 1",
                params![req.wing, req.hall, req.room, req.content],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }

        // Dedup check: first 100 chars match in same wing+hall+room (catches near-duplicates)
        // Uses SUBSTR instead of LIKE to avoid metachar injection (%, _)
        if req.content.chars().count() >= 50 {
            let prefix: String = req.content.chars().take(100).collect();
            let prefix_char_count = prefix.chars().count() as i64;
            let existing: Option<i64> = conn
                .query_row(
                    "SELECT id FROM memory_nodes WHERE wing = ?1 AND hall = ?2 AND room = ?3 AND SUBSTR(content, 1, ?4) = ?5 LIMIT 1",
                    params![req.wing, req.hall, req.room, prefix_char_count, prefix],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(id) = existing {
                return Ok(id);
            }
        }

        conn.execute(
            "INSERT INTO memory_nodes (wing, room, hall, content, summary, source, importance)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                req.wing,
                req.room,
                req.hall,
                req.content,
                req.summary,
                req.source,
                req.importance.unwrap_or(5),
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Write a memory node that is part of a chunked sequence.
    pub fn memory_write_chunk(
        &self,
        req: &WriteRequest,
        chunk_index: i32,
        parent_id: Option<i64>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO memory_nodes (wing, room, hall, content, summary, source, importance, chunk_index, parent_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                req.wing,
                req.room,
                req.hall,
                req.content,
                req.summary,
                req.source,
                req.importance.unwrap_or(5),
                chunk_index,
                parent_id,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Delete a memory node (and its embedding).
    pub fn memory_delete(&self, wing: &str, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute(
            "DELETE FROM memory_nodes WHERE id = ?1 AND wing = ?2",
            params![id, wing],
        )?;
        if deleted > 0 {
            // Also remove from vector index (ignore errors if not present)
            let _ = conn.execute(
                "DELETE FROM vec_memories WHERE node_id = ?1",
                params![id],
            );
        }
        Ok(())
    }

    /// Get a single memory node by ID.
    #[allow(dead_code)]
    pub fn memory_get(&self, id: i64) -> Result<Option<MemoryNode>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, wing, room, hall, content, summary, source, importance,
                    chunk_index, parent_id, created_at, updated_at, metadata
             FROM memory_nodes WHERE id = ?1",
        )?;
        let node = stmt
            .query_row(params![id], |row| {
                Ok(MemoryNode {
                    id: row.get(0)?,
                    wing: row.get(1)?,
                    room: row.get(2)?,
                    hall: row.get(3)?,
                    content: row.get(4)?,
                    summary: row.get(5)?,
                    source: row.get(6)?,
                    importance: row.get(7)?,
                    chunk_index: row.get(8)?,
                    parent_id: row.get(9)?,
                    created_at: row.get(10)?,
                    updated_at: row.get(11)?,
                    metadata: row.get(12)?,
                })
            })
            .optional()?;
        Ok(node)
    }

    /// List all wings with memory counts.
    pub fn memory_list_wings(&self) -> Result<Vec<WingInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT wing, COUNT(*) FROM memory_nodes GROUP BY wing ORDER BY wing",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(WingInfo {
                name: row.get(0)?,
                count: row.get::<_, i64>(1)? as usize,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// List rooms in a wing with counts.
    pub fn memory_list_rooms(&self, wing: &str) -> Result<Vec<RoomInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT room, COUNT(*) FROM memory_nodes WHERE wing = ?1 GROUP BY room ORDER BY room",
        )?;
        let rows = stmt.query_map(params![wing], |row| {
            Ok(RoomInfo {
                name: row.get(0)?,
                count: row.get::<_, i64>(1)? as usize,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Get palace status for a specific wing.
    pub fn memory_status(&self, wing: &str) -> Result<PalaceStatus> {
        let conn = self.conn.lock().unwrap();

        let total_memories: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_nodes WHERE wing = ?1",
            params![wing],
            |row| row.get(0),
        )?;

        // Rooms
        let mut stmt = conn.prepare(
            "SELECT room, COUNT(*) FROM memory_nodes WHERE wing = ?1 GROUP BY room ORDER BY room",
        )?;
        let rooms: Vec<RoomInfo> = stmt
            .query_map(params![wing], |row| {
                Ok(RoomInfo {
                    name: row.get(0)?,
                    count: row.get::<_, i64>(1)? as usize,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Hall counts
        let mut stmt = conn.prepare(
            "SELECT hall, COUNT(*) FROM memory_nodes WHERE wing = ?1 GROUP BY hall ORDER BY hall",
        )?;
        let hall_counts: Vec<(String, usize)> = stmt
            .query_map(params![wing], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // KG counts
        let kg_entities: i64 = conn.query_row(
            "SELECT COUNT(*) FROM kg_entities WHERE wing = ?1",
            params![wing],
            |row| row.get(0),
        )?;
        let kg_triples: i64 = conn.query_row(
            "SELECT COUNT(*) FROM kg_triples WHERE wing = ?1 AND valid_to IS NULL",
            params![wing],
            |row| row.get(0),
        )?;

        Ok(PalaceStatus {
            wing: wing.to_string(),
            total_memories: total_memories as usize,
            rooms,
            hall_counts,
            kg_entities: kg_entities as usize,
            kg_triples: kg_triples as usize,
        })
    }

    /// Get top-importance memories for L1 context generation.
    /// Get top memories for L1 context, scored by importance * recency.
    /// Recency decay: score = importance + 2.0 / (1 + days_since_creation).
    /// This gives recent high-importance memories priority over old ones.
    pub fn memory_top_important(
        &self,
        wing: &str,
        min_importance: i32,
        limit: usize,
    ) -> Result<Vec<MemoryNode>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, wing, room, hall, content, summary, source, importance,
                    chunk_index, parent_id, created_at, updated_at, metadata
             FROM memory_nodes
             WHERE wing = ?1 AND importance >= ?2 AND chunk_index IS NULL
             ORDER BY (importance + 2.0 / (1.0 + julianday('now') - julianday(created_at))) DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![wing, min_importance, limit as i64], |row| {
            Ok(MemoryNode {
                id: row.get(0)?,
                wing: row.get(1)?,
                room: row.get(2)?,
                hall: row.get(3)?,
                content: row.get(4)?,
                summary: row.get(5)?,
                source: row.get(6)?,
                importance: row.get(7)?,
                chunk_index: row.get(8)?,
                parent_id: row.get(9)?,
                created_at: row.get(10)?,
                updated_at: row.get(11)?,
                metadata: row.get(12)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Get a palace_meta value.
    pub fn palace_meta_get(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let val = conn
            .query_row(
                "SELECT value FROM palace_meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(val)
    }

    /// Set a palace_meta value (upsert).
    pub fn palace_meta_set(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO palace_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Insert an embedding vector for a memory node.
    pub fn memory_insert_embedding(&self, node_id: i64, embedding: &[f32]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let bytes: &[u8] = zerocopy::IntoBytes::as_bytes(embedding);
        conn.execute(
            "INSERT INTO vec_memories (node_id, embedding) VALUES (?1, ?2)",
            params![node_id, bytes],
        )?;
        Ok(())
    }

    /// Delete an embedding for a memory node.
    #[allow(dead_code)]
    pub fn memory_delete_embedding(&self, node_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM vec_memories WHERE node_id = ?1",
            params![node_id],
        )?;
        Ok(())
    }

    /// FTS5 search — returns (node_id, bm25_rank) pairs.
    /// `wing`: if Some, filter by wing; if None, search all wings.
    pub fn memory_search_fts(
        &self,
        wing: Option<&str>,
        query: &str,
        room: Option<&str>,
        hall: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(i64, f64)>> {
        let conn = self.conn.lock().unwrap();

        // Build WHERE clause for filtering
        let mut conditions: Vec<String> = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(w) = wing {
            conditions.push("wing = ?".to_string());
            param_values.push(Box::new(w.to_string()));
        }
        if let Some(r) = room {
            conditions.push("room = ?".to_string());
            param_values.push(Box::new(r.to_string()));
        }
        if let Some(h) = hall {
            conditions.push("hall = ?".to_string());
            param_values.push(Box::new(h.to_string()));
        }

        let subquery = if conditions.is_empty() {
            "SELECT id FROM memory_nodes".to_string()
        } else {
            format!(
                "SELECT id FROM memory_nodes WHERE {}",
                conditions.join(" AND ")
            )
        };

        let sql = format!(
            "SELECT rowid, rank FROM memory_nodes_fts
             WHERE memory_nodes_fts MATCH ?
               AND rowid IN ({})
             ORDER BY rank
             LIMIT ?",
            subquery
        );

        // params: [query, wing, room?, hall?, limit]
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(query.to_string())];
        all_params.extend(param_values);
        all_params.push(Box::new(limit as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Vector search via sqlite-vec — returns (node_id, distance) pairs.
    /// Lower distance = more similar (cosine distance).
    /// `wing`: if Some, filter by wing; if None, search all wings.
    pub fn memory_search_vec(
        &self,
        embedding: &[f32],
        wing: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(i64, f64)>> {
        let conn = self.conn.lock().unwrap();
        let bytes: &[u8] = zerocopy::IntoBytes::as_bytes(embedding);

        if let Some(w) = wing {
            let mut stmt = conn.prepare(
                "SELECT v.node_id, v.distance
                 FROM vec_memories v
                 JOIN memory_nodes m ON m.id = v.node_id
                 WHERE v.embedding MATCH ?1
                   AND k = ?2
                   AND m.wing = ?3
                 ORDER BY v.distance",
            )?;
            let rows = stmt.query_map(params![bytes, limit as i64, w], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        } else {
            let mut stmt = conn.prepare(
                "SELECT v.node_id, v.distance
                 FROM vec_memories v
                 WHERE v.embedding MATCH ?1
                   AND k = ?2
                 ORDER BY v.distance",
            )?;
            let rows = stmt.query_map(params![bytes, limit as i64], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        }
    }

    /// Fetch full MemoryNode rows by IDs (for assembling search results).
    pub fn memory_get_batch(&self, ids: &[i64]) -> Result<Vec<MemoryNode>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let conn = self.conn.lock().unwrap();
        let placeholders: String = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT id, wing, room, hall, content, summary, source, importance,
                    chunk_index, parent_id, created_at, updated_at, metadata
             FROM memory_nodes WHERE id IN ({})",
            placeholders
        );
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<Box<dyn rusqlite::types::ToSql>> =
            ids.iter().map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>).collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(MemoryNode {
                id: row.get(0)?,
                wing: row.get(1)?,
                room: row.get(2)?,
                hall: row.get(3)?,
                content: row.get(4)?,
                summary: row.get(5)?,
                source: row.get(6)?,
                importance: row.get(7)?,
                chunk_index: row.get(8)?,
                parent_id: row.get(9)?,
                created_at: row.get(10)?,
                updated_at: row.get(11)?,
                metadata: row.get(12)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Update a memory node's summary and room (used by Haiku post-processing).
    /// Update a memory node's summary, room, and optionally importance.
    pub fn memory_update_analysis(
        &self,
        id: i64,
        summary: &str,
        room: &str,
        importance: Option<i32>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        if let Some(imp) = importance {
            conn.execute(
                "UPDATE memory_nodes SET summary = ?1, room = ?2, importance = ?3, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?4",
                params![summary, room, imp, id],
            )?;
        } else {
            conn.execute(
                "UPDATE memory_nodes SET summary = ?1, room = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?3",
                params![summary, room, id],
            )?;
        }
        Ok(())
    }

    /// Find rooms that span multiple wings (tunnels).
    pub fn memory_find_tunnels(&self) -> Result<Vec<TunnelInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT room, GROUP_CONCAT(DISTINCT wing) as wings
             FROM memory_nodes
             GROUP BY room
             HAVING COUNT(DISTINCT wing) > 1
             ORDER BY room",
        )?;
        let rows = stmt.query_map([], |row| {
            let room: String = row.get(0)?;
            let wings_str: String = row.get(1)?;
            let wings: Vec<String> = wings_str.split(',').map(|s| s.to_string()).collect();
            Ok(TunnelInfo { room, wings })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// List room names for a wing (for Haiku context — just names, no counts).
    #[allow(dead_code)]
    pub fn memory_room_names(&self, wing: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT room FROM memory_nodes WHERE wing = ?1 ORDER BY room",
        )?;
        let rows = stmt.query_map(params![wing], |row| row.get(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Reassign all memory nodes in `old_room` to `new_room`, clearing summaries
    /// (so backfill re-analyzes them). Returns count of affected rows.
    pub fn memory_reassign_room(&self, old_room: &str, new_room: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count = conn.execute(
            "UPDATE memory_nodes SET room = ?1, summary = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE room = ?2",
            params![new_room, old_room],
        )?;
        Ok(count)
    }

    /// List all room names across ALL wings (for Haiku cross-wing room context).
    /// Returns deduplicated, sorted room names so Haiku can reuse names consistently.
    pub fn memory_all_room_names(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT room FROM memory_nodes ORDER BY room",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Get top KG triples for L1 context (current facts only, limited).
    pub fn kg_top_triples(&self, wing: &str, limit: usize) -> Result<Vec<crate::memory::Triple>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT t.id, s.name, t.predicate, o.name, t.confidence, t.valid_from, t.valid_to
             FROM kg_triples t
             JOIN kg_entities s ON t.subject_id = s.id
             JOIN kg_entities o ON t.object_id = o.id
             WHERE t.wing = ?1 AND t.valid_to IS NULL
             ORDER BY t.confidence DESC, t.created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![wing, limit as i64], |row| {
            let valid_to: Option<String> = row.get(6)?;
            Ok(crate::memory::Triple {
                id: row.get(0)?,
                subject: row.get(1)?,
                predicate: row.get(2)?,
                object: row.get(3)?,
                confidence: row.get(4)?,
                valid_from: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                current: valid_to.is_none(),
                valid_to,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Get all entity names for a wing (for KG hint matching).
    pub fn kg_entity_names(&self, wing: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT name FROM kg_entities WHERE wing = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map(params![wing], |row| row.get(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
}
