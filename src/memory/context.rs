use crate::error::Result;
use crate::state::StateDb;

/// Build L1 wake-up context from the palace DB.
/// Returns formatted markdown to inject into the system prompt.
/// Includes top-importance memories (recency-weighted) and KG relationship summary.
pub fn build_l1_context(db: &StateDb, wing: &str, max_chars: usize) -> Result<String> {
    let nodes = db.memory_top_important(wing, 7, 20)?;
    let triples = db.kg_top_triples(wing, 15)?;

    if nodes.is_empty() && triples.is_empty() {
        return Ok(String::new());
    }

    let mut output = String::from("\n# Memory (auto-loaded from palace)\n");
    let mut total_chars = output.len();

    // KG relationship summary (compact, high information density)
    if !triples.is_empty() {
        let header = "\n## Known Relationships\n";
        output.push_str(header);
        total_chars += header.len();

        for triple in &triples {
            let line = format!("- {} → {} → {}\n", triple.subject, triple.predicate, triple.object);
            if total_chars + line.len() > max_chars {
                break;
            }
            output.push_str(&line);
            total_chars += line.len();
        }
    }

    // Memory nodes grouped by hall
    if !nodes.is_empty() {
        let mut current_hall = String::new();

        for node in &nodes {
            if total_chars >= max_chars {
                break;
            }

            // Group by hall
            if node.hall != current_hall {
                let header = format!("\n## {}\n", capitalize(&node.hall));
                if total_chars + header.len() > max_chars {
                    break;
                }
                output.push_str(&header);
                total_chars += header.len();
                current_hall = node.hall.clone();
            }

            // Use summary if available, otherwise truncate content
            let text = if let Some(ref summary) = node.summary {
                if summary.is_empty() {
                    truncate_content(&node.content, 200)
                } else {
                    summary.clone()
                }
            } else {
                truncate_content(&node.content, 200)
            };

            let line = format!("- [{}] {}\n", node.room, text);
            if total_chars + line.len() > max_chars {
                break;
            }
            output.push_str(&line);
            total_chars += line.len();
        }
    }

    Ok(output)
}

fn truncate_content(content: &str, max_len: usize) -> String {
    if content.len() <= max_len {
        content.to_string()
    } else {
        // Find last valid char boundary at or before max_len to avoid panics on CJK text
        let mut boundary = max_len;
        while boundary > 0 && !content.is_char_boundary(boundary) {
            boundary -= 1;
        }
        format!("{}...", &content[..boundary])
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}
