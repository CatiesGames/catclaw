use std::sync::Arc;

use tokio::process::Command;
use tracing::warn;

use crate::error::{CatClawError, Result};
use crate::memory::embed::Embedder;
use crate::memory::{DiaryAnalysis, WriteRequest};
use crate::state::StateDb;

const HAIKU_MODEL: &str = "claude-haiku-4-5-20251001";

const ANALYZE_PROMPT: &str = r#"You are a memory analyst. Given a diary entry, extract structured data.

Existing rooms (shared across all agents): {rooms_list}

Diary entry:
{diary_text}

Respond in JSON only (no markdown fences, no explanation):
{
  "summary": "One sentence summary of this diary entry",
  "room": "primary-room for the diary entry itself",
  "diary_importance": 5,
  "facts": [
    {
      "content": "verbatim fact or preference",
      "room": "room for THIS fact (may differ from diary room)",
      "hall": "facts|preferences|discoveries|advice",
      "importance": 7,
      "subject": "entity name",
      "predicate": "relationship",
      "object": "entity name"
    }
  ]
}

Rules:
- summary: one sentence, preserve key details, same language as diary
- room: pick from existing rooms if applicable, else create new (kebab-case). Use "general" only as last resort.
- Each fact has its OWN room — a diary about both coding and a novel should put coding facts in "coding" and novel facts in "cyberpunk-novel".
- facts: only extract genuinely important/reusable information. Skip trivial details.
- hall: facts=objective truths, preferences=likes/dislikes, discoveries=insights, advice=lessons
- Importance scale (apply to both diary_importance and each fact):
  9-10: Identity-level — user name, role, core project, language preference
  7-8: Work-level — coding style, tool preferences, project tech stack, key decisions
  5-6: Reference — specific discussion outcomes, bug fixes, routine work
  3-4: Background — general events, casual conversations
- subject/predicate/object: for knowledge graph triples, use lowercase. Omit if not applicable.
- If no facts worth extracting, return empty facts array
- If the content is too short, just a header, or has no meaningful information, return: {"summary": "", "room": "general", "diary_importance": 1, "facts": []}"#;

/// Analyze a diary entry using Haiku to extract summary, room, and facts.
/// Runs as a background task after diary extraction — failures are non-fatal.
pub async fn analyze_diary(
    state_db: &StateDb,
    embedder: Option<&Arc<tokio::sync::OnceCell<Embedder>>>,
    wing: &str,
    diary_node_id: i64,
    diary_text: &str,
) -> Result<()> {
    // 1. Fetch existing rooms across ALL wings for consistent naming (enables tunnels)
    let rooms = state_db.memory_all_room_names()?;
    let rooms_list = if rooms.is_empty() {
        "none yet".to_string()
    } else {
        rooms.join(", ")
    };

    // 2. Call Haiku subprocess
    let analysis = call_haiku(diary_text, &rooms_list).await?;

    // 3. If Haiku returned empty summary, content is not meaningful — delete the node
    if analysis.summary.is_empty() && analysis.facts.is_empty() {
        let _ = state_db.memory_delete(wing, diary_node_id);
        return Ok(());
    }

    // 4. Update diary node with summary + corrected room + Haiku-assessed importance
    state_db.memory_update_analysis(
        diary_node_id,
        &analysis.summary,
        &analysis.room,
        Some(analysis.diary_importance.clamp(1, 10)),
    )?;

    // 5. Generate embedding for the diary node
    if let Some(emb_cell) = embedder {
        if let Some(emb) = emb_cell.get() {
            match emb.embed_one(diary_text).await {
                Ok(vec) => {
                    let _ = state_db.memory_insert_embedding(diary_node_id, &vec);
                }
                Err(e) => warn!(error = %e, "analyze: diary embedding failed"),
            }
        }
    }

    // 5. Write each extracted fact as a separate memory node + KG triple
    for fact in &analysis.facts {
        let hall = validate_hall(&fact.hall);
        let fact_room = fact
            .room
            .as_deref()
            .filter(|r| !r.is_empty())
            .unwrap_or(&analysis.room);
        let req = WriteRequest {
            wing: wing.to_string(),
            room: fact_room.to_string(),
            hall,
            content: fact.content.clone(),
            summary: None,
            source: "extraction".to_string(),
            importance: Some(fact.importance.clamp(1, 10)),
        };
        let fact_id = state_db.memory_write(&req)?;

        // Create KG triple if entity info provided
        if let (Some(subj), Some(pred), Some(obj)) =
            (&fact.subject, &fact.predicate, &fact.object)
        {
            if !subj.is_empty() && !pred.is_empty() && !obj.is_empty() {
                let sub_id = state_db.kg_get_or_create_entity(wing, subj, None)?;
                let obj_id = state_db.kg_get_or_create_entity(wing, obj, None)?;
                let _ = state_db.kg_add_triple(wing, sub_id, pred, obj_id, None, 1.0);
            }
        }

        // Generate embedding for the fact
        if let Some(emb_cell) = embedder {
            if let Some(emb) = emb_cell.get() {
                match emb.embed_one(&fact.content).await {
                    Ok(vec) => {
                        let _ = state_db.memory_insert_embedding(fact_id, &vec);
                    }
                    Err(e) => warn!(error = %e, "analyze: fact embedding failed"),
                }
            }
        }
    }

    tracing::info!(
        wing,
        diary_node_id,
        summary_len = analysis.summary.len(),
        room = %analysis.room,
        facts = analysis.facts.len(),
        "diary analysis complete"
    );

    Ok(())
}

/// Lightweight post-processing for agent-written memories (via memory_write).
/// Classifies room (if "general") + generates summary (if long) + embedding.
/// No fact extraction — content written by agent is already a fact.
/// `original_importance`: the importance value set at write time. If != 5 (the default),
/// it means the agent deliberately chose the importance, so Haiku should not override it.
pub async fn classify_memory(
    state_db: &StateDb,
    embedder: Option<&Arc<tokio::sync::OnceCell<Embedder>>>,
    node_id: i64,
    content: &str,
    current_room: &str,
    original_importance: i32,
) -> Result<()> {
    let needs_room = current_room == "general";
    let needs_summary = content.len() >= 200;

    // Call Haiku for room classification (if "general") and/or summary (if long) and importance adjustment
    if needs_room || needs_summary {
        let rooms = state_db.memory_all_room_names()?;
        let rooms_list = if rooms.is_empty() {
            "none yet".to_string()
        } else {
            rooms.join(", ")
        };

        match call_haiku(content, &rooms_list).await {
            Ok(analysis) => {
                let new_room = if needs_room { &analysis.room } else { current_room };
                let summary = if needs_summary { &analysis.summary } else { "" };
                // Only override importance if agent used the default (5) — respect agent-set values
                let importance = if original_importance == 5 {
                    Some(analysis.diary_importance.clamp(1, 10))
                } else {
                    None
                };
                state_db.memory_update_analysis(node_id, summary, new_room, importance)?;
            }
            Err(e) => {
                warn!(error = %e, "classify_memory: haiku failed (non-fatal)");
            }
        }
    }

    // Always generate embedding
    if let Some(emb_cell) = embedder {
        if let Some(emb) = emb_cell.get() {
            match emb.embed_one(content).await {
                Ok(vec) => {
                    let _ = state_db.memory_insert_embedding(node_id, &vec);
                }
                Err(e) => warn!(error = %e, "classify_memory: embedding failed"),
            }
        }
    }

    Ok(())
}

/// Call Haiku model to analyze diary text. Returns parsed DiaryAnalysis.
async fn call_haiku(diary_text: &str, rooms_list: &str) -> Result<DiaryAnalysis> {
    let prompt = ANALYZE_PROMPT
        .replace("{diary_text}", diary_text)
        .replace("{rooms_list}", rooms_list);

    let result = Command::new("claude")
        .args([
            "-p",
            &prompt,
            "--model",
            HAIKU_MODEL,
            "--max-turns",
            "1",
            "--output-format",
            "text",
            "--dangerously-skip-permissions",
            "--tools",
            "",
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env_remove("CLAUDECODE")
        .output()
        .await;

    match result {
        Ok(output) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(CatClawError::Memory(format!(
                    "haiku exited with {}: {}",
                    output.status, stderr
                )));
            }
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            parse_analysis(&text)
        }
        Err(e) => Err(CatClawError::Memory(format!(
            "failed to spawn haiku: {}",
            e
        ))),
    }
}

/// Parse JSON output from Haiku, handling potential markdown fences.
fn parse_analysis(text: &str) -> Result<DiaryAnalysis> {
    // Strip markdown code fences if present
    let json_str = text
        .trim()
        .strip_prefix("```json")
        .or_else(|| text.trim().strip_prefix("```"))
        .unwrap_or(text.trim());
    let json_str = json_str
        .strip_suffix("```")
        .unwrap_or(json_str)
        .trim();

    serde_json::from_str(json_str).map_err(|e| {
        let preview: String = text.chars().take(100).collect();
        CatClawError::Memory(format!(
            "failed to parse haiku JSON: {}. Raw output: {}",
            e, preview
        ))
    })
}

/// Validate hall value, falling back to "facts" for invalid values.
fn validate_hall(hall: &str) -> String {
    match hall {
        "facts" | "events" | "discoveries" | "preferences" | "advice" => hall.to_string(),
        _ => "facts".to_string(),
    }
}
