use std::collections::HashMap;

use crate::error::Result;
use crate::memory::{SearchResult, embed::Embedder};
use crate::state::StateDb;

/// RRF constant (Reciprocal Rank Fusion). Higher k smooths rank differences.
const RRF_K: f64 = 60.0;

/// Hybrid search combining FTS5 full-text and sqlite-vec cosine similarity.
/// Results are merged using Reciprocal Rank Fusion (RRF).
/// `wing`: if Some, search only that wing; if None, search all wings (cross_wing).
pub async fn hybrid_search(
    db: &StateDb,
    embedder: Option<&Embedder>,
    wing: Option<&str>,
    query: &str,
    room: Option<&str>,
    hall: Option<&str>,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    let fetch_limit = limit * 3; // Over-fetch for better merge results

    // Path A: FTS5 full-text search
    let fts_results = db.memory_search_fts(wing, query, room, hall, fetch_limit).unwrap_or_default();

    // Path B: Vector cosine similarity (if embedder available)
    let vec_results = if let Some(emb) = embedder {
        if let Ok(query_vec) = emb.embed_one(query).await {
            db.memory_search_vec(&query_vec, wing, fetch_limit).unwrap_or_default()
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    // Merge with RRF
    let merged_ids = rrf_merge(&fts_results, &vec_results, limit);

    if merged_ids.is_empty() {
        return Ok(vec![]);
    }

    // Fetch full nodes
    let ids: Vec<i64> = merged_ids.iter().map(|(id, _)| *id).collect();
    let nodes = db.memory_get_batch(&ids)?;

    // Build score map
    let score_map: HashMap<i64, f64> = merged_ids.into_iter().collect();

    // Assemble results in score order
    let mut results: Vec<SearchResult> = nodes
        .into_iter()
        .map(|node| {
            let score = score_map.get(&node.id).copied().unwrap_or(0.0);
            SearchResult {
                id: node.id,
                wing: node.wing,
                room: node.room,
                hall: node.hall,
                content: node.content,
                summary: node.summary,
                importance: node.importance,
                created_at: node.created_at,
                score,
            }
        })
        .collect();

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);
    Ok(results)
}

/// Reciprocal Rank Fusion: merge two ranked lists into one.
/// score(d) = sum( 1 / (k + rank_in_list) ) for each list containing d.
fn rrf_merge(
    fts_results: &[(i64, f64)],
    vec_results: &[(i64, f64)],
    limit: usize,
) -> Vec<(i64, f64)> {
    let mut scores: HashMap<i64, f64> = HashMap::new();

    for (rank, (id, _)) in fts_results.iter().enumerate() {
        *scores.entry(*id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
    }

    for (rank, (id, _)) in vec_results.iter().enumerate() {
        *scores.entry(*id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
    }

    let mut merged: Vec<(i64, f64)> = scores.into_iter().collect();
    merged.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    merged.truncate(limit);
    merged
}
