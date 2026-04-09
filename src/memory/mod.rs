mod db;
pub mod analyze;
pub mod embed;
pub mod search;
pub mod kg;
pub mod tools;
pub mod context;
pub mod migrate;

/// A request to write a memory node into the palace.
#[derive(Debug, Clone)]
pub struct WriteRequest {
    pub wing: String,
    pub room: String,
    pub hall: String,
    pub content: String,
    pub summary: Option<String>,
    pub source: String,
    pub importance: Option<i32>,
}

impl Default for WriteRequest {
    fn default() -> Self {
        Self {
            wing: String::new(),
            room: "general".to_string(),
            hall: "facts".to_string(),
            content: String::new(),
            summary: None,
            source: "agent".to_string(),
            importance: None,
        }
    }
}

/// A memory node stored in the palace.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MemoryNode {
    pub id: i64,
    pub wing: String,
    pub room: String,
    pub hall: String,
    pub content: String,
    pub summary: Option<String>,
    pub source: String,
    pub importance: i32,
    pub chunk_index: Option<i32>,
    pub parent_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    pub metadata: Option<String>,
}

/// Search result with combined relevance score.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SearchResult {
    pub id: i64,
    pub wing: String,
    pub room: String,
    pub hall: String,
    pub content: String,
    pub summary: Option<String>,
    pub importance: i32,
    pub created_at: String,
    pub score: f64,
}

/// Wing summary info.
#[derive(Debug, Clone)]
pub struct WingInfo {
    pub name: String,
    pub count: usize,
}

/// Room summary info.
#[derive(Debug, Clone)]
pub struct RoomInfo {
    pub name: String,
    pub count: usize,
}

/// Palace status for a specific wing.
#[derive(Debug, Clone)]
pub struct PalaceStatus {
    pub wing: String,
    pub total_memories: usize,
    pub rooms: Vec<RoomInfo>,
    pub hall_counts: Vec<(String, usize)>,
    pub kg_entities: usize,
    pub kg_triples: usize,
}

/// A knowledge graph triple.
#[derive(Debug, Clone)]
pub struct Triple {
    pub id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f64,
    pub valid_from: String,
    pub valid_to: Option<String>,
    pub current: bool,
}

/// Result of Haiku diary analysis (closet + fact extraction + room classification).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DiaryAnalysis {
    pub summary: String,
    pub room: String,
    /// Haiku-assessed importance for the diary entry itself.
    #[serde(default = "default_importance")]
    pub diary_importance: i32,
    #[serde(default)]
    pub facts: Vec<ExtractedFact>,
}

/// A fact extracted from a diary entry by Haiku.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ExtractedFact {
    pub content: String,
    /// Room for this specific fact (may differ from the diary's primary room).
    #[serde(default)]
    pub room: Option<String>,
    pub hall: String,
    #[serde(default = "default_importance")]
    pub importance: i32,
    pub subject: Option<String>,
    pub predicate: Option<String>,
    pub object: Option<String>,
}

fn default_importance() -> i32 {
    7
}

/// A tunnel: a room that exists across multiple wings.
#[derive(Debug, Clone)]
pub struct TunnelInfo {
    pub room: String,
    pub wings: Vec<String>,
}

/// Chunking constants (MemPalace defaults).
pub const CHUNK_SIZE: usize = 800;
pub const CHUNK_OVERLAP: usize = 100;
pub const MIN_CHUNK_SIZE: usize = 50;

/// Find the last valid UTF-8 char boundary at or before `pos`.
fn safe_boundary(text: &str, pos: usize) -> usize {
    if pos >= text.len() {
        return text.len();
    }
    let mut p = pos;
    while p > 0 && !text.is_char_boundary(p) {
        p -= 1;
    }
    p
}

/// Split text into overlapping chunks for storage and embedding.
/// All byte offsets are snapped to char boundaries to avoid panics on CJK text.
pub fn chunk_text(text: &str) -> Vec<String> {
    if text.len() <= CHUNK_SIZE {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < text.len() {
        let end = safe_boundary(text, (start + CHUNK_SIZE).min(text.len()));

        // Try to break at paragraph boundary
        let actual_end = if end < text.len() {
            let slice = &text[start..end];
            if let Some(pos) = slice.rfind("\n\n") {
                if pos > MIN_CHUNK_SIZE {
                    start + pos + 2
                } else if let Some(pos) = slice.rfind('\n') {
                    if pos > MIN_CHUNK_SIZE {
                        start + pos + 1
                    } else {
                        end
                    }
                } else {
                    end
                }
            } else if let Some(pos) = slice.rfind('\n') {
                if pos > MIN_CHUNK_SIZE {
                    start + pos + 1
                } else {
                    end
                }
            } else {
                end
            }
        } else {
            end
        };

        let chunk = text[start..actual_end].trim().to_string();
        if chunk.len() >= MIN_CHUNK_SIZE {
            chunks.push(chunk);
        }

        // Move start back by overlap for next chunk
        if actual_end >= text.len() {
            break;
        }
        start = safe_boundary(text, if actual_end > CHUNK_OVERLAP {
            actual_end - CHUNK_OVERLAP
        } else {
            actual_end
        });
    }

    chunks
}
