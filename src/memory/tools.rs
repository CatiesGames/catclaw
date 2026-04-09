use std::sync::Arc;

use serde_json::Value;

use crate::error::{CatClawError, Result};
use crate::memory::embed::Embedder;
use crate::memory::search::hybrid_search;
use crate::memory::WriteRequest;
use crate::state::StateDb;

/// Build MCP tool definitions for memory palace tools.
pub fn build_memory_tools() -> Vec<Value> {
    vec![
        tool(
            "memory_status",
            "Show palace overview: rooms, halls, memory counts, and KG stats for the specified wing.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "wing": {"type": "string", "description": "Wing name (usually your agent ID)"}
                },
                "required": ["wing"]
            }),
        ),
        tool(
            "memory_write",
            "Store a memory in the palace. Use halls: facts (objective truths), events (what happened), discoveries (insights), preferences (likes/dislikes), advice (lessons learned). Set importance 8-10 for critical info.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "wing": {"type": "string", "description": "Wing name (usually your agent ID)"},
                    "content": {"type": "string", "description": "The memory content to store (verbatim)"},
                    "room": {"type": "string", "description": "Topic grouping (e.g. 'auth-system', 'user-prefs')", "default": "general"},
                    "hall": {"type": "string", "enum": ["facts", "events", "discoveries", "preferences", "advice"], "default": "facts"},
                    "summary": {"type": "string", "description": "Optional condensed summary"},
                    "importance": {"type": "integer", "minimum": 1, "maximum": 10, "default": 5, "description": "1-10 scale. 8+ appears in boot context."}
                },
                "required": ["wing", "content"]
            }),
        ),
        tool(
            "memory_search",
            "Search memories using hybrid full-text + semantic search. Always search before answering about past work, decisions, or preferences.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "wing": {"type": "string", "description": "Wing name to search in"},
                    "query": {"type": "string", "description": "Search query"},
                    "room": {"type": "string", "description": "Filter by room (optional)"},
                    "hall": {"type": "string", "description": "Filter by hall (optional)"},
                    "limit": {"type": "integer", "default": 5, "description": "Max results to return"},
                    "cross_wing": {"type": "boolean", "default": false, "description": "Search across ALL wings (use only when memory_tunnels suggests shared knowledge)"}
                },
                "required": ["wing", "query"]
            }),
        ),
        tool(
            "memory_delete",
            "Delete a memory by ID.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "wing": {"type": "string", "description": "Wing name"},
                    "id": {"type": "integer", "description": "Memory node ID to delete"}
                },
                "required": ["wing", "id"]
            }),
        ),
        tool(
            "memory_list_wings",
            "List all wings (agent/project namespaces) with memory counts.",
            serde_json::json!({"type": "object", "properties": {}, "required": []}),
        ),
        tool(
            "memory_list_rooms",
            "List rooms (topics) in a wing with memory counts.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "wing": {"type": "string", "description": "Wing name"}
                },
                "required": ["wing"]
            }),
        ),
        tool(
            "kg_add",
            "Add a fact triple to the knowledge graph (e.g. 'user' 'prefers' 'Rust').",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "wing": {"type": "string", "description": "Wing name"},
                    "subject": {"type": "string", "description": "Subject entity"},
                    "predicate": {"type": "string", "description": "Relationship (e.g. prefers, uses, works_on)"},
                    "object": {"type": "string", "description": "Object entity"},
                    "valid_from": {"type": "string", "description": "When this fact became true (ISO 8601, optional)"},
                    "confidence": {"type": "number", "minimum": 0, "maximum": 1, "default": 1.0}
                },
                "required": ["wing", "subject", "predicate", "object"]
            }),
        ),
        tool(
            "kg_invalidate",
            "Mark a fact as no longer valid (set expiry date).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "wing": {"type": "string", "description": "Wing name"},
                    "subject": {"type": "string", "description": "Subject entity"},
                    "predicate": {"type": "string", "description": "Relationship"},
                    "object": {"type": "string", "description": "Object entity"},
                    "valid_until": {"type": "string", "description": "When this fact stopped being true (ISO 8601, defaults to now)"}
                },
                "required": ["wing", "subject", "predicate", "object"]
            }),
        ),
        tool(
            "kg_query",
            "Query all known facts about an entity. Verify facts before stating them.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "wing": {"type": "string", "description": "Wing name"},
                    "entity": {"type": "string", "description": "Entity name to query"},
                    "as_of": {"type": "string", "description": "Only return facts valid at this time (ISO 8601, optional)"},
                    "direction": {"type": "string", "enum": ["outgoing", "incoming", "both"], "default": "both"}
                },
                "required": ["wing", "entity"]
            }),
        ),
        tool(
            "kg_timeline",
            "Get facts in chronological order, optionally filtered by entity.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "wing": {"type": "string", "description": "Wing name"},
                    "entity": {"type": "string", "description": "Filter by entity (optional)"}
                },
                "required": ["wing"]
            }),
        ),
        tool(
            "memory_tunnels",
            "Find rooms that exist across multiple wings (agents). Tunnels reveal shared knowledge areas — use with cross_wing search to explore them.",
            serde_json::json!({"type": "object", "properties": {}, "required": []}),
        ),
    ]
}

/// Execute a memory or kg tool call.
pub async fn execute_memory_tool(
    state_db: &Arc<StateDb>,
    embedder: &Arc<tokio::sync::OnceCell<Embedder>>,
    tool_name: &str,
    args: Value,
) -> Result<Value> {
    match tool_name {
        "memory_status" => {
            let wing = str_arg(&args, "wing")?;
            let status = state_db.memory_status(wing)?;

            let halls: Vec<String> = status
                .hall_counts
                .iter()
                .map(|(h, c)| format!("{}({})", h, c))
                .collect();
            let rooms: Vec<String> = status
                .rooms
                .iter()
                .map(|r| format!("{}: {}", r.name, r.count))
                .collect();

            let overview = format!(
                "Palace Status — wing \"{}\":\n\
                 Total memories: {} across {} rooms\n\
                 Halls: {}\n\
                 Rooms: {}\n\
                 KG: {} entities, {} active triples",
                status.wing,
                status.total_memories,
                status.rooms.len(),
                halls.join(" "),
                rooms.join(", "),
                status.kg_entities,
                status.kg_triples,
            );
            Ok(serde_json::json!({"status": overview}))
        }

        "memory_write" => {
            let wing = str_arg(&args, "wing")?;
            let content = str_arg(&args, "content")?;
            let room = args.get("room").and_then(|v| v.as_str()).unwrap_or("general");
            let hall = args.get("hall").and_then(|v| v.as_str()).unwrap_or("facts");
            let summary = args.get("summary").and_then(|v| v.as_str()).map(String::from);
            let importance = args.get("importance").and_then(|v| v.as_i64()).map(|v| v as i32);
            let source = args.get("source").and_then(|v| v.as_str()).unwrap_or("agent");

            let chunks = crate::memory::chunk_text(content);
            let is_chunked = chunks.len() > 1;
            let mut first_id = None;
            let mut node_ids = Vec::new();

            for (i, chunk) in chunks.iter().enumerate() {
                let req = WriteRequest {
                    wing: wing.to_string(),
                    room: room.to_string(),
                    hall: hall.to_string(),
                    content: chunk.clone(),
                    summary: if i == 0 { summary.clone() } else { None },
                    source: source.to_string(),
                    importance,
                };

                let id = if is_chunked {
                    state_db.memory_write_chunk(&req, i as i32, first_id)?
                } else {
                    state_db.memory_write(&req)?
                };

                if i == 0 {
                    first_id = Some(id);
                }
                node_ids.push(id);
            }

            // Background: Haiku classification (room + summary) + embedding generation
            let db = state_db.clone();
            let emb = embedder.clone();
            let primary_id = first_id.unwrap_or(0);
            let content_for_classify = chunks.first().cloned().unwrap_or_default();
            let room_for_classify = room.to_string();
            let orig_importance = importance.unwrap_or(5);
            tokio::spawn(async move {
                // Classify the primary node (room + summary)
                if primary_id > 0 {
                    let _ = crate::memory::analyze::classify_memory(
                        &db,
                        Some(&emb),
                        primary_id,
                        &content_for_classify,
                        &room_for_classify,
                        orig_importance,
                    )
                    .await;
                }
            });

            Ok(serde_json::json!({
                "success": true,
                "id": primary_id,
                "chunks": node_ids.len(),
                "wing": wing,
                "room": room,
                "hall": hall,
            }))
        }

        "memory_search" => {
            let wing = str_arg(&args, "wing")?;
            let query = str_arg(&args, "query")?;
            let room = args.get("room").and_then(|v| v.as_str());
            let hall = args.get("hall").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
            let cross_wing = args.get("cross_wing").and_then(|v| v.as_bool()).unwrap_or(false);

            let search_wing = if cross_wing { None } else { Some(wing) };

            let results = hybrid_search(
                state_db,
                embedder.get(),
                search_wing,
                query,
                room,
                hall,
                limit,
            )
            .await?;

            let items: Vec<Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.id,
                        "room": r.room,
                        "hall": r.hall,
                        "content": r.content,
                        "summary": r.summary,
                        "importance": r.importance,
                        "created_at": r.created_at,
                        "score": r.score,
                    })
                })
                .collect();

            // KG hint: find known entities that appear in search results
            let kg_hint = find_kg_entities_in_results(state_db, wing, &results);

            let mut response = serde_json::json!({
                "query": query,
                "wing": wing,
                "results": items,
                "count": items.len(),
            });
            if !kg_hint.is_empty() {
                response["kg_entities_found"] = serde_json::json!(kg_hint);
                response["kg_hint"] = serde_json::json!(
                    "These entities exist in your knowledge graph. Use kg_query to explore their relationships."
                );
            }

            Ok(response)
        }

        "memory_delete" => {
            let wing = str_arg(&args, "wing")?;
            let id = args
                .get("id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| CatClawError::Memory("missing 'id' parameter".to_string()))?;
            state_db.memory_delete(wing, id)?;
            Ok(serde_json::json!({"success": true, "deleted_id": id}))
        }

        "memory_list_wings" => {
            let wings = state_db.memory_list_wings()?;
            let items: Vec<Value> = wings
                .iter()
                .map(|w| serde_json::json!({"wing": w.name, "count": w.count}))
                .collect();
            Ok(serde_json::json!({"wings": items}))
        }

        "memory_list_rooms" => {
            let wing = str_arg(&args, "wing")?;
            let rooms = state_db.memory_list_rooms(wing)?;
            let items: Vec<Value> = rooms
                .iter()
                .map(|r| serde_json::json!({"room": r.name, "count": r.count}))
                .collect();
            Ok(serde_json::json!({"wing": wing, "rooms": items}))
        }

        "kg_add" => {
            let wing = str_arg(&args, "wing")?;
            let subject = str_arg(&args, "subject")?;
            let predicate = str_arg(&args, "predicate")?;
            let object = str_arg(&args, "object")?;
            let valid_from = args.get("valid_from").and_then(|v| v.as_str());
            let confidence = args.get("confidence").and_then(|v| v.as_f64()).unwrap_or(1.0);

            let sub_id = state_db.kg_get_or_create_entity(wing, subject, None)?;
            let obj_id = state_db.kg_get_or_create_entity(wing, object, None)?;
            let triple_id =
                state_db.kg_add_triple(wing, sub_id, predicate, obj_id, valid_from, confidence)?;

            Ok(serde_json::json!({
                "success": true,
                "triple_id": triple_id,
                "fact": format!("{} {} {}", subject, predicate, object),
            }))
        }

        "kg_invalidate" => {
            let wing = str_arg(&args, "wing")?;
            let subject = str_arg(&args, "subject")?;
            let predicate = str_arg(&args, "predicate")?;
            let object = str_arg(&args, "object")?;
            let valid_until = args
                .get("valid_until")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let now = chrono::Utc::now().to_rfc3339();
            let until = if valid_until.is_empty() { &now } else { valid_until };

            let updated = state_db.kg_invalidate(wing, subject, predicate, object, until)?;
            Ok(serde_json::json!({
                "success": true,
                "updated": updated,
                "fact": format!("{} {} {}", subject, predicate, object),
                "ended": until,
            }))
        }

        "kg_query" => {
            let wing = str_arg(&args, "wing")?;
            let entity = str_arg(&args, "entity")?;
            let as_of = args.get("as_of").and_then(|v| v.as_str());
            let direction = args
                .get("direction")
                .and_then(|v| v.as_str())
                .unwrap_or("both");

            let triples = state_db.kg_query_entity(wing, entity, as_of, direction)?;
            let items: Vec<Value> = triples
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "id": t.id,
                        "subject": t.subject,
                        "predicate": t.predicate,
                        "object": t.object,
                        "confidence": t.confidence,
                        "valid_from": t.valid_from,
                        "valid_to": t.valid_to,
                        "current": t.current,
                    })
                })
                .collect();

            Ok(serde_json::json!({
                "entity": entity,
                "wing": wing,
                "facts": items,
                "count": items.len(),
            }))
        }

        "kg_timeline" => {
            let wing = str_arg(&args, "wing")?;
            let entity = args.get("entity").and_then(|v| v.as_str());

            let triples = state_db.kg_timeline(wing, entity)?;
            let items: Vec<Value> = triples
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "subject": t.subject,
                        "predicate": t.predicate,
                        "object": t.object,
                        "valid_from": t.valid_from,
                        "valid_to": t.valid_to,
                        "current": t.current,
                    })
                })
                .collect();

            Ok(serde_json::json!({
                "wing": wing,
                "timeline": items,
                "count": items.len(),
            }))
        }

        "memory_tunnels" => {
            let tunnels = state_db.memory_find_tunnels()?;
            let items: Vec<Value> = tunnels
                .iter()
                .map(|t| serde_json::json!({"room": t.room, "wings": t.wings}))
                .collect();
            Ok(serde_json::json!({
                "tunnels": items,
                "count": items.len(),
            }))
        }

        _ => Err(CatClawError::Memory(format!(
            "unknown memory tool: {}",
            tool_name
        ))),
    }
}

fn tool(name: &str, description: &str, schema: Value) -> Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "inputSchema": schema,
    })
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| CatClawError::Memory(format!("missing '{}' parameter", key)))
}

/// Find known KG entity names that appear in search result content.
fn find_kg_entities_in_results(
    state_db: &StateDb,
    wing: &str,
    results: &[crate::memory::SearchResult],
) -> Vec<String> {
    let entities = match state_db.kg_entity_names(wing) {
        Ok(e) => e,
        Err(_) => return vec![],
    };
    if entities.is_empty() || results.is_empty() {
        return vec![];
    }

    // Combine all result content into one string for matching
    let combined: String = results
        .iter()
        .map(|r| r.content.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let combined_lower = combined.to_lowercase();

    // Find entities whose name appears in the combined content.
    // Entity names are stored normalized (lowercase, spaces→underscores),
    // so check both underscore and space forms against the content.
    entities
        .into_iter()
        .filter(|name| {
            let with_spaces = name.replace('_', " ");
            combined_lower.contains(name.as_str()) || combined_lower.contains(&with_spaces)
        })
        .collect()
}
