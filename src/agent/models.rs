/// Known Claude CLI model short names and their full model IDs.
pub const KNOWN_MODELS: &[(&str, &str)] = &[
    ("opus", "claude-opus-4-6"),
    ("sonnet", "claude-sonnet-4-6"),
    ("haiku", "claude-haiku-4-5-20251001"),
];

/// Resolve a model name: short name → full name, unknown names pass through.
pub fn resolve_model(name: &str) -> String {
    let name = name.trim();
    for &(short, full) in KNOWN_MODELS {
        if name.eq_ignore_ascii_case(short) {
            return full.to_string();
        }
    }
    name.to_string()
}

/// Get a display-friendly short name for a full model ID.
/// Returns the short name if found, otherwise the original string.
pub fn model_display_name(full: &str) -> &str {
    for &(short, full_name) in KNOWN_MODELS {
        if full == full_name {
            return short;
        }
    }
    full
}
