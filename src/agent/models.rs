//! Model identifier handling: parsing, resolution, and the canonical
//! `provider/model` form used by catclaw config files and WS protocols.
//!
//! All model strings stored in `catclaw.toml` or sent over WS are now in
//! `provider/model` form — e.g. `claude/opus-4-7`, `codex/gpt-5.4-mini`.
//! `Config::load` migrates old un-prefixed values (e.g. `opus`, `haiku`,
//! `claude-opus-4-8`) on first load by writing them back with the
//! `claude/` prefix preserved.
//!
//! The downstream CLI args builder (`claude_args_with_mcp` /
//! `codex_args_from`) reads `ProviderModel.model` to get the raw model ID
//! without the prefix — both CLIs expect bare model names.

use crate::agent::Runtime;

/// A model identifier resolved to its `(provider, full_model_id)` parts.
///
/// Constructed from a `provider/model` string or a legacy un-prefixed alias.
/// `model` is always the full upstream model ID (e.g. `claude-opus-4-8`,
/// `gpt-5.4-mini`), never an alias — aliases are resolved during parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderModel {
    pub provider: Runtime,
    pub model: String,
}

impl ProviderModel {
    /// Canonical wire form: `provider/model`. Stable across save/load cycles.
    #[allow(dead_code)] // used by C.2 migration + C.6 TUI rendering
    pub fn to_wire_string(&self) -> String {
        format!("{}/{}", self.provider.as_str(), self.model)
    }
}

/// Known model catalog. Aliases (shorter names that resolve to a full ID) are
/// listed too — `parse_model_string` walks both alias and full-id lookups.
pub struct ModelEntry {
    pub provider: Runtime,
    /// Short alias (e.g. "opus", "haiku") or canonical short name. May equal
    /// `full_id` for entries that have no alias.
    pub alias: &'static str,
    /// Full upstream model ID that the CLI consumes (e.g. claude-opus-4-7).
    pub full_id: &'static str,
    /// Display description for TUI picker.
    pub description: &'static str,
}

#[allow(dead_code)]
pub const KNOWN_MODELS: &[ModelEntry] = &[
    // ── Claude ────────────────────────────────────────────────────────────
    ModelEntry {
        provider: Runtime::Claude,
        alias: "opus-4-8",
        full_id: "claude-opus-4-8",
        description: "Opus 4.8 — newest flagship, 1M context, Fast mode",
    },
    ModelEntry {
        provider: Runtime::Claude,
        alias: "opus-4-7",
        full_id: "claude-opus-4-7",
        description: "Opus 4.7 — previous flagship",
    },
    ModelEntry {
        provider: Runtime::Claude,
        alias: "opus-4-6",
        full_id: "claude-opus-4-6",
        description: "Opus 4.6 — older flagship",
    },
    ModelEntry {
        provider: Runtime::Claude,
        alias: "sonnet-5",
        full_id: "claude-sonnet-5",
        description: "Sonnet 5 — newest balanced, near-Opus coding/agentic",
    },
    ModelEntry {
        provider: Runtime::Claude,
        alias: "sonnet-4-6",
        full_id: "claude-sonnet-4-6",
        description: "Sonnet 4.6 — previous balanced",
    },
    ModelEntry {
        provider: Runtime::Claude,
        alias: "haiku-4-5",
        full_id: "claude-haiku-4-5-20251001",
        description: "Haiku 4.5 — fastest, cheapest",
    },
    // Short top-level aliases (Claude is the default provider, so `opus`
    // alone — no `claude/` prefix — resolves here for back-compat).
    ModelEntry {
        provider: Runtime::Claude,
        alias: "opus",
        full_id: "claude-opus-4-8",
        description: "alias → claude/opus-4-8",
    },
    ModelEntry {
        provider: Runtime::Claude,
        alias: "sonnet",
        full_id: "claude-sonnet-5",
        description: "alias → claude/sonnet-5",
    },
    ModelEntry {
        provider: Runtime::Claude,
        alias: "haiku",
        full_id: "claude-haiku-4-5-20251001",
        description: "alias → claude/haiku-4-5",
    },
    // ── Codex (OpenAI) ─────────────────────────────────────────────────────
    // Codex CLI's `-c model="..."` / `-m` accepts any model name the ChatGPT
    // account is entitled to — the actual set depends on the user's plan, not
    // on this list. These are completion HINTS only; `parse_model_string` lets
    // codex/* pass through unchanged so users can type any id codex accepts.
    // Verified available on a standard ChatGPT-account Codex login (2026-06):
    // gpt-5.5, gpt-5.4, gpt-5.4-mini, o3. NOTE: `gpt-5.5-mini` is NOT available
    // on ChatGPT-account Codex (400 "model is not supported") — do not re-add
    // it as a hint; use gpt-5.4-mini for the cheap tier.
    ModelEntry {
        provider: Runtime::Codex,
        alias: "gpt-5.5",
        full_id: "gpt-5.5",
        description: "GPT-5.5 — current flagship",
    },
    ModelEntry {
        provider: Runtime::Codex,
        alias: "gpt-5.4",
        full_id: "gpt-5.4",
        description: "GPT-5.4 — previous flagship, balanced",
    },
    ModelEntry {
        provider: Runtime::Codex,
        alias: "gpt-5.4-mini",
        full_id: "gpt-5.4-mini",
        description: "GPT-5.4 mini — fastest, cheapest",
    },
    ModelEntry {
        provider: Runtime::Codex,
        alias: "o3",
        full_id: "o3",
        description: "o3 — reasoning",
    },
];

/// Canonical default model (wire form) for a runtime. Used when switching an
/// agent's runtime: the prior `claude/*` model can't be carried over to a codex
/// runtime (and vice versa), so we reset to the runtime's flagship default.
pub fn default_model_for_runtime(runtime: Runtime) -> &'static str {
    match runtime {
        Runtime::Claude => "claude/opus-4-8",
        Runtime::Codex => "codex/gpt-5.5",
    }
}

/// Parse a model string in any of the supported forms into a [`ProviderModel`]:
///
/// 1. Canonical: `claude/opus-4-7` / `codex/gpt-5.4-mini`
/// 2. Provider + alias: `claude/opus` / `codex/mini`
/// 3. Legacy un-prefixed alias: `opus` / `haiku` (defaults to claude)
/// 4. Legacy full ID: `claude-opus-4-8` / `gpt-5.5` (provider sniffed from prefix)
///
/// Returns `Err` for malformed strings (e.g. `unknown/foo`).
pub fn parse_model_string(s: &str) -> Result<ProviderModel, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("model string is empty".to_string());
    }

    // Canonical / provider+alias form.
    if let Some((provider_str, model_part)) = s.split_once('/') {
        let provider = match provider_str.trim().to_lowercase().as_str() {
            "claude" => Runtime::Claude,
            "codex" => Runtime::Codex,
            other => {
                return Err(format!(
                    "unknown provider '{}' (expected 'claude' or 'codex')",
                    other
                ));
            }
        };
        let model_part = model_part.trim();
        if model_part.is_empty() {
            return Err(format!("model string '{}' has empty model part", s));
        }
        // Resolve alias if any (for the matching provider only).
        let resolved = KNOWN_MODELS
            .iter()
            .find(|m| m.provider == provider && m.alias.eq_ignore_ascii_case(model_part))
            .map(|m| m.full_id.to_string())
            .unwrap_or_else(|| model_part.to_string());
        return Ok(ProviderModel {
            provider,
            model: resolved,
        });
    }

    // Legacy un-prefixed: try alias resolution across all providers (claude
    // wins ties because its entries are listed first).
    if let Some(entry) = KNOWN_MODELS
        .iter()
        .find(|m| m.alias.eq_ignore_ascii_case(s))
    {
        return Ok(ProviderModel {
            provider: entry.provider,
            model: entry.full_id.to_string(),
        });
    }

    // Legacy full ID: provider sniff by prefix.
    let lower = s.to_lowercase();
    if lower.starts_with("claude-") {
        return Ok(ProviderModel {
            provider: Runtime::Claude,
            model: s.to_string(),
        });
    }
    if lower.starts_with("gpt-")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
        || lower.starts_with("codex-")
    {
        return Ok(ProviderModel {
            provider: Runtime::Codex,
            model: s.to_string(),
        });
    }

    Err(format!(
        "unrecognised model '{}' — use `claude/<name>` or `codex/<name>` form",
        s
    ))
}

/// Get a display-friendly short name for a full model ID (back-compat helper
/// used by TUI rendering — returns the alias if one is registered, otherwise
/// the original full ID unchanged).
#[allow(dead_code)]
pub fn model_display_name(full: &str) -> &str {
    for entry in KNOWN_MODELS {
        if entry.full_id == full && entry.alias != entry.full_id {
            return entry.alias;
        }
    }
    full
}

/// Resolve a model string to its raw upstream model ID, dropping provider
/// prefix and resolving aliases. Used by callers that already know which CLI
/// to spawn and just want the bare name to pass via `--model` / `-c model=`.
///
/// Falls back to the input on unparseable strings so existing call sites that
/// trusted `resolve_model` to not panic keep working.
#[allow(dead_code)]
pub fn resolve_model(name: &str) -> String {
    match parse_model_string(name) {
        Ok(pm) => pm.model,
        Err(_) => name.to_string(),
    }
}
