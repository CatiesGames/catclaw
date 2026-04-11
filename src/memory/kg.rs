use rusqlite::{params, OptionalExtension};

use crate::error::Result;
use crate::memory::Triple;
use crate::state::StateDb;

impl StateDb {
    /// Get or create an entity by wing + name. Returns the entity ID.
    pub fn kg_get_or_create_entity(
        &self,
        wing: &str,
        name: &str,
        entity_type: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let normalized = name.to_lowercase().replace(' ', "_").replace('\'', "");

        // Try to find existing
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM kg_entities WHERE wing = ?1 AND name = ?2",
                params![wing, normalized],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(id) = existing {
            return Ok(id);
        }

        // Create new
        conn.execute(
            "INSERT INTO kg_entities (wing, name, entity_type) VALUES (?1, ?2, ?3)",
            params![wing, normalized, entity_type.unwrap_or("unknown")],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Add a fact triple. Returns the triple ID.
    /// If an identical active triple already exists, returns its ID without duplicating.
    #[allow(clippy::too_many_arguments)]
    pub fn kg_add_triple(
        &self,
        wing: &str,
        subject_id: i64,
        predicate: &str,
        object_id: i64,
        valid_from: Option<&str>,
        confidence: f64,
        source_node_id: Option<i64>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let pred = predicate.to_lowercase().replace(' ', "_");

        // Check for existing active triple
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM kg_triples
                 WHERE wing = ?1 AND subject_id = ?2 AND predicate = ?3
                   AND object_id = ?4 AND valid_to IS NULL",
                params![wing, subject_id, pred, object_id],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(id) = existing {
            return Ok(id);
        }

        conn.execute(
            "INSERT INTO kg_triples (wing, subject_id, predicate, object_id, confidence, source_node_id, valid_from)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, COALESCE(?7, strftime('%Y-%m-%dT%H:%M:%fZ','now')))",
            params![wing, subject_id, pred, object_id, confidence, source_node_id, valid_from],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Invalidate (expire) a fact triple by setting valid_to.
    /// Returns the number of rows updated.
    pub fn kg_invalidate(
        &self,
        wing: &str,
        subject: &str,
        predicate: &str,
        object: &str,
        valid_until: &str,
    ) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let sub = subject.to_lowercase().replace(' ', "_").replace('\'', "");
        let obj = object.to_lowercase().replace(' ', "_").replace('\'', "");
        let pred = predicate.to_lowercase().replace(' ', "_");

        let updated = conn.execute(
            "UPDATE kg_triples SET valid_to = ?1
             WHERE wing = ?2 AND valid_to IS NULL
               AND subject_id IN (SELECT id FROM kg_entities WHERE wing = ?2 AND name = ?3)
               AND predicate = ?4
               AND object_id IN (SELECT id FROM kg_entities WHERE wing = ?2 AND name = ?5)",
            params![valid_until, wing, sub, pred, obj],
        )?;
        Ok(updated)
    }

    /// Query all facts related to an entity.
    /// `direction`: "outgoing" (entity → ?), "incoming" (? → entity), "both"
    /// `as_of`: if provided, only return facts valid at that time.
    pub fn kg_query_entity(
        &self,
        wing: &str,
        entity: &str,
        as_of: Option<&str>,
        direction: &str,
    ) -> Result<Vec<Triple>> {
        let conn = self.conn.lock().unwrap();
        let ename = entity.to_lowercase().replace(' ', "_").replace('\'', "");
        let mut results = Vec::new();

        if direction == "outgoing" || direction == "both" {
            let mut sql = String::from(
                "SELECT t.id, s.name, t.predicate, o.name, t.confidence, t.valid_from, t.valid_to
                 FROM kg_triples t
                 JOIN kg_entities s ON t.subject_id = s.id
                 JOIN kg_entities o ON t.object_id = o.id
                 WHERE t.wing = ?1
                   AND s.name = ?2",
            );
            if let Some(ts) = as_of {
                sql.push_str(
                    " AND (t.valid_from IS NULL OR t.valid_from <= ?3)
                      AND (t.valid_to IS NULL OR t.valid_to >= ?3)",
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![wing, ename, ts], row_to_triple)?;
                for row in rows {
                    results.push(row?);
                }
            } else {
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![wing, ename], row_to_triple)?;
                for row in rows {
                    results.push(row?);
                }
            }
        }

        if direction == "incoming" || direction == "both" {
            let mut sql = String::from(
                "SELECT t.id, s.name, t.predicate, o.name, t.confidence, t.valid_from, t.valid_to
                 FROM kg_triples t
                 JOIN kg_entities s ON t.subject_id = s.id
                 JOIN kg_entities o ON t.object_id = o.id
                 WHERE t.wing = ?1
                   AND o.name = ?2",
            );
            if let Some(ts) = as_of {
                sql.push_str(
                    " AND (t.valid_from IS NULL OR t.valid_from <= ?3)
                      AND (t.valid_to IS NULL OR t.valid_to >= ?3)",
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![wing, ename, ts], row_to_triple)?;
                for row in rows {
                    results.push(row?);
                }
            } else {
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![wing, ename], row_to_triple)?;
                for row in rows {
                    results.push(row?);
                }
            }
        }

        Ok(results)
    }

    /// Get facts in chronological order, optionally filtered by entity.
    pub fn kg_timeline(
        &self,
        wing: &str,
        entity: Option<&str>,
    ) -> Result<Vec<Triple>> {
        let conn = self.conn.lock().unwrap();

        let (sql, use_entity) = if let Some(e) = entity {
            let ename = e.to_lowercase().replace(' ', "_").replace('\'', "");
            (
                "SELECT t.id, s.name, t.predicate, o.name, t.confidence, t.valid_from, t.valid_to
                     FROM kg_triples t
                     JOIN kg_entities s ON t.subject_id = s.id
                     JOIN kg_entities o ON t.object_id = o.id
                     WHERE t.wing = ?1 AND (s.name = ?2 OR o.name = ?2)
                     ORDER BY t.valid_from ASC NULLS LAST
                     LIMIT 100"
                    .to_string(),
                Some(ename),
            )
        } else {
            (
                "SELECT t.id, s.name, t.predicate, o.name, t.confidence, t.valid_from, t.valid_to
                 FROM kg_triples t
                 JOIN kg_entities s ON t.subject_id = s.id
                 JOIN kg_entities o ON t.object_id = o.id
                 WHERE t.wing = ?1
                 ORDER BY t.valid_from ASC NULLS LAST
                 LIMIT 100"
                    .to_string(),
                None,
            )
        };

        let mut stmt = conn.prepare(&sql)?;
        let rows = if let Some(ref ename) = use_entity {
            stmt.query_map(params![wing, ename], row_to_triple)?
        } else {
            stmt.query_map(params![wing], row_to_triple)?
        };

        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
}

/// Helper to convert a row into a Triple.
fn row_to_triple(row: &rusqlite::Row) -> rusqlite::Result<Triple> {
    let valid_to: Option<String> = row.get(6)?;
    Ok(Triple {
        id: row.get(0)?,
        subject: row.get(1)?,
        predicate: row.get(2)?,
        object: row.get(3)?,
        confidence: row.get(4)?,
        valid_from: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
        current: valid_to.is_none(),
        valid_to,
    })
}
