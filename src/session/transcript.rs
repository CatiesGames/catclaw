use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tracing::warn;

use crate::error::Result;

/// A single entry in a session transcript (JSONL format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub timestamp: String,
    pub role: String, // "user" | "assistant" | "system"
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use: Option<Vec<ToolUseEntry>>,
    /// Channel metadata (only on system entries)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUseEntry {
    pub name: String,
    pub input: serde_json::Value,
}

/// Sidecar state describing the latest diary marker recorded in the transcript.
/// Persists across restarts; read by the diary scheduler / rolling trigger to
/// avoid full-file scans and to enforce failure back-off.
///
/// Stored as `{transcript_path}.marker` (JSON, one-shot overwrite).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MarkerState {
    /// File offset (bytes) immediately AFTER the last diary marker line.
    /// Entries with byte position >= this offset are "since last marker".
    /// 0 means no marker recorded yet (read from the start of the file).
    #[serde(default)]
    pub byte_offset: u64,
    /// Number of user turns recorded since the last diary marker.
    /// Used by the rolling diary trigger ("write a diary every N user turns").
    #[serde(default)]
    pub user_turns_since: u32,
    /// Kind of the last diary marker, if any. None when the transcript has
    /// never had a diary marker written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_marker_kind: Option<MarkerKind>,
    /// Unix seconds when the last marker was written.
    #[serde(default)]
    pub last_marker_unix: i64,
    /// Consecutive failure count — drives exponential back-off. Reset to 0
    /// on the next successful extraction or skip.
    #[serde(default)]
    pub fail_attempt: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerKind {
    Extracted,
    Skipped,
    Failed,
}

impl MarkerKind {
    fn from_content(content: &str) -> Option<Self> {
        if content.starts_with("diary_extracted") {
            Some(MarkerKind::Extracted)
        } else if content.starts_with("diary_skipped") {
            Some(MarkerKind::Skipped)
        } else if content.starts_with("diary_failed") {
            Some(MarkerKind::Failed)
        } else {
            None
        }
    }
}

/// Manages transcript files for a session, stored under the agent's workspace.
///
/// Layout: {agent_workspace}/transcripts/{label}_{session_id}.jsonl
/// Falls back to {session_id}.jsonl for backward compatibility.
///
/// Each transcript has a `{path}.marker` JSON sidecar tracking the latest
/// diary marker offset + user-turn count since that marker. The sidecar lets
/// `read_since_last_marker` seek directly to new content instead of re-reading
/// the entire JSONL on every scheduler tick (the historic cause of multi-GiB
/// disk-read spikes — see CLAUDE.md lesson on diary-trigger disk reads).
pub struct TranscriptLog {
    path: PathBuf,
}

#[allow(dead_code)]
impl TranscriptLog {
    /// Open (or create) a transcript log for a session.
    /// If `label` is provided and no existing file is found, creates `{label}_{session_id}.jsonl`.
    /// Falls back to `{session_id}.jsonl` for backward compatibility with existing transcripts.
    pub async fn open(agent_workspace: &Path, session_id: &str) -> Result<Self> {
        Self::open_with_label(agent_workspace, session_id, None).await
    }

    /// Open with an explicit label (e.g. "discord_general" from the session key).
    pub async fn open_with_label(
        agent_workspace: &Path,
        session_id: &str,
        label: Option<&str>,
    ) -> Result<Self> {
        let dir = agent_workspace.join("transcripts");
        fs::create_dir_all(&dir).await?;

        // Try to find an existing file matching this session_id (any label prefix)
        let plain = dir.join(format!("{}.jsonl", session_id));
        if plain.exists() {
            return Ok(TranscriptLog { path: plain });
        }

        // Check for existing labeled file via glob: *_{session_id}.jsonl
        if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
            let suffix = format!("_{}.jsonl", session_id);
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Some(name) = entry.file_name().to_str() {
                    if name.ends_with(&suffix) {
                        return Ok(TranscriptLog { path: entry.path() });
                    }
                }
            }
        }

        // Create new file with label if provided
        let filename = if let Some(lbl) = label {
            let safe = sanitize_label(lbl);
            if safe.is_empty() {
                format!("{}.jsonl", session_id)
            } else {
                format!("{}_{}.jsonl", safe, session_id)
            }
        } else {
            format!("{}.jsonl", session_id)
        };
        let path = dir.join(filename);
        Ok(TranscriptLog { path })
    }

    /// Open an existing transcript file for a session. Returns None if no file exists.
    /// Does NOT create a new file — use this for read-only operations like diary extraction.
    pub async fn open_existing(agent_workspace: &Path, session_id: &str) -> Option<Self> {
        let dir = agent_workspace.join("transcripts");

        // Try plain {session_id}.jsonl
        let plain = dir.join(format!("{}.jsonl", session_id));
        if plain.exists() {
            return Some(TranscriptLog { path: plain });
        }

        // Try labeled *_{session_id}.jsonl
        let suffix = format!("_{}.jsonl", session_id);
        if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Some(name) = entry.file_name().to_str() {
                    if name.ends_with(&suffix) {
                        return Some(TranscriptLog { path: entry.path() });
                    }
                }
            }
        }

        None
    }

    /// Log session start with channel metadata
    pub async fn log_session_start(
        &self,
        session_key: &str,
        channel_type: Option<&str>,
        channel_id: Option<&str>,
        channel_name: Option<&str>,
    ) {
        let entry = TranscriptEntry {
            timestamp: Utc::now().to_rfc3339(),
            role: "system".to_string(),
            content: "session_created".to_string(),
            sender_id: None,
            sender_name: None,
            tool_use: None,
            channel_type: channel_type.map(String::from),
            channel_id: channel_id.map(String::from),
            channel_name: channel_name.map(String::from),
            session_key: Some(session_key.to_string()),
        };
        self.append(&entry).await;
    }

    /// Append a user message to the transcript. Returns the new
    /// `user_turns_since` count from the sidecar (post-increment) so callers
    /// can decide whether to fire the rolling-diary trigger.
    pub async fn log_user(
        &self,
        message: &str,
        sender_id: Option<&str>,
        sender_name: Option<&str>,
    ) -> u32 {
        let entry = TranscriptEntry {
            timestamp: Utc::now().to_rfc3339(),
            role: "user".to_string(),
            content: message.to_string(),
            sender_id: sender_id.map(String::from),
            sender_name: sender_name.map(String::from),
            tool_use: None,
            channel_type: None,
            channel_id: None,
            channel_name: None,
            session_key: None,
        };
        self.append(&entry).await;
        // Increment the sidecar's user-turn counter.
        let mut state = self.read_marker_state().await;
        state.user_turns_since = state.user_turns_since.saturating_add(1);
        let count = state.user_turns_since;
        self.write_marker_state(&state).await;
        count
    }

    /// Append an assistant response to the transcript
    pub async fn log_assistant(&self, response: &str, tools: Option<Vec<ToolUseEntry>>) {
        let entry = TranscriptEntry {
            timestamp: Utc::now().to_rfc3339(),
            role: "assistant".to_string(),
            content: response.to_string(),
            sender_id: None,
            sender_name: None,
            tool_use: tools,
            channel_type: None,
            channel_id: None,
            channel_name: None,
            session_key: None,
        };
        self.append(&entry).await;
    }

    /// Append a system event to the transcript. When `event` is a diary
    /// marker (`diary_extracted` / `diary_skipped` / `diary_failed:...`) the
    /// sidecar is updated so the next `read_since_last_marker` skips straight
    /// to new content.
    pub async fn log_system(&self, event: &str) {
        let entry = TranscriptEntry {
            timestamp: Utc::now().to_rfc3339(),
            role: "system".to_string(),
            content: event.to_string(),
            sender_id: None,
            sender_name: None,
            tool_use: None,
            channel_type: None,
            channel_id: None,
            channel_name: None,
            session_key: None,
        };
        let new_len = self.append(&entry).await;

        // If this was a diary marker, update the sidecar so future reads can
        // skip everything before it.
        //
        // **Crucial distinction by marker kind:**
        // - `Extracted` / `Skipped`: we successfully consumed everything up
        //   to this point → advance `byte_offset` to EOF and reset turn
        //   counter + fail counter. Next read sees only newer entries.
        // - `Failed`: the turns are STILL unprocessed and must be retried
        //   when back-off elapses. Do NOT advance `byte_offset` — that
        //   would make the next read return zero entries and the retry
        //   would silently skip (`marker_found && entries.is_empty()` in
        //   `extract_diary_for_session`), permanently losing those turns.
        //   The Failed marker content embeds the attempt number
        //   (`diary_failed:{n}:{rfc3339}`) so a sidecar rebuild can recover
        //   it from the JSONL — without that, a gateway restart that loses
        //   the sidecar would reset `fail_attempt` to 0 and collapse a
        //   6-hour back-off back to 5 minutes.
        if let Some(kind) = MarkerKind::from_content(event) {
            if let Some(byte_offset) = new_len {
                let mut state = self.read_marker_state().await;
                match kind {
                    MarkerKind::Extracted | MarkerKind::Skipped => {
                        state.byte_offset = byte_offset;
                        state.user_turns_since = 0;
                        state.last_marker_kind = Some(kind);
                        state.last_marker_unix = Utc::now().timestamp();
                        state.fail_attempt = 0;
                    }
                    MarkerKind::Failed => {
                        // Keep byte_offset and user_turns_since pointing at the
                        // pre-failure state so retry can see the same turns.
                        // Use the attempt number encoded in `event` (caller
                        // computed `previous + 1`) so re-reading the JSONL
                        // recovers the same value.
                        state.last_marker_kind = Some(kind);
                        state.last_marker_unix = Utc::now().timestamp();
                        state.fail_attempt = parse_diary_failed_attempt(event)
                            .unwrap_or_else(|| state.fail_attempt.saturating_add(1));
                    }
                }
                self.write_marker_state(&state).await;
            }
        }
    }

    /// Read all entries from the transcript
    pub async fn read_all(&self) -> Vec<TranscriptEntry> {
        let content = match fs::read_to_string(&self.path).await {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        content
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() {
                    return None;
                }
                match serde_json::from_str::<TranscriptEntry>(line) {
                    Ok(entry) => Some(entry),
                    Err(e) => {
                        warn!(error = %e, "failed to parse transcript entry");
                        None
                    }
                }
            })
            .collect()
    }

    /// Read the last N entries from the transcript
    pub async fn read_last(&self, n: usize) -> Vec<TranscriptEntry> {
        let all = self.read_all().await;
        let start = all.len().saturating_sub(n);
        all[start..].to_vec()
    }

    /// Format transcript entries as readable text (for summaries)
    pub fn format_readable(entries: &[TranscriptEntry]) -> String {
        let mut out = String::new();
        for entry in entries {
            let name = match &entry.sender_name {
                Some(n) => n.as_str(),
                None if entry.role == "assistant" => "Assistant",
                None => &entry.role,
            };
            out.push_str(&format!("[{}] {}: {}\n", entry.timestamp, name, entry.content));
        }
        out
    }

    /// Read entries since the last diary marker (`diary_extracted` /
    /// `diary_skipped` / `diary_failed`).
    ///
    /// Returns `(entries_after_marker, marker_was_found)`. Uses the `.marker`
    /// sidecar's `byte_offset` to seek directly to new content. If the sidecar
    /// is missing or stale (offset past end-of-file), falls back to a full read
    /// and rebuilds the sidecar so subsequent reads stay cheap.
    pub async fn read_since_last_marker(&self) -> (Vec<TranscriptEntry>, bool) {
        let state = self.read_marker_state().await;
        let file_len = match fs::metadata(&self.path).await {
            Ok(m) => m.len(),
            Err(_) => return (Vec::new(), state.last_marker_kind.is_some()),
        };

        // Validate sidecar offset; fall back to full scan + rebuild if stale.
        if state.byte_offset == 0 || state.byte_offset > file_len {
            let (entries, found) = self.full_scan_and_rebuild_sidecar(state.byte_offset > file_len).await;
            return (filter_diary_failed_markers(entries), found);
        }

        // Fast path: read only the tail.
        let entries = match self.read_range(state.byte_offset, file_len).await {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "transcript: tail read failed, falling back to full scan");
                let (entries, found) = self.full_scan_and_rebuild_sidecar(true).await;
                return (filter_diary_failed_markers(entries), found);
            }
        };
        // Strip `diary_failed:*` markers from the visible window: when retry
        // happens after back-off, byte_offset still points at the pre-failure
        // position so the LLM can re-see the unprocessed turns, but the
        // intervening Failed marker(s) shouldn't be sent to the LLM as noise.
        (filter_diary_failed_markers(entries), state.last_marker_kind.is_some())
    }

    /// Load the marker sidecar. Returns Default if missing or unparseable.
    pub async fn read_marker_state(&self) -> MarkerState {
        let sidecar = self.marker_sidecar_path();
        let content = match fs::read_to_string(&sidecar).await {
            Ok(c) => c,
            Err(_) => return MarkerState::default(),
        };
        serde_json::from_str(&content).unwrap_or_default()
    }

    fn marker_sidecar_path(&self) -> PathBuf {
        let mut p = self.path.clone().into_os_string();
        p.push(".marker");
        PathBuf::from(p)
    }

    /// Atomically persist the sidecar (write to tmp then rename). Best-effort:
    /// errors are logged but do not propagate — the only consequence of a lost
    /// sidecar is one full-scan rebuild on next read.
    async fn write_marker_state(&self, state: &MarkerState) {
        let sidecar = self.marker_sidecar_path();
        let json = match serde_json::to_vec(state) {
            Ok(j) => j,
            Err(e) => {
                warn!(error = %e, "transcript: failed to serialize marker state");
                return;
            }
        };
        let tmp = {
            let mut p = sidecar.clone().into_os_string();
            p.push(".tmp");
            PathBuf::from(p)
        };
        if let Err(e) = fs::write(&tmp, &json).await {
            warn!(error = %e, path = %tmp.display(), "transcript: failed to write marker sidecar tmp");
            return;
        }
        if let Err(e) = fs::rename(&tmp, &sidecar).await {
            warn!(error = %e, path = %sidecar.display(), "transcript: failed to rename marker sidecar");
        }
    }

    /// Read JSONL entries between [start, end) byte offsets. The starting
    /// offset is expected to land on a line boundary (we always advance to the
    /// next `\n` to be defensive against misaligned offsets).
    async fn read_range(&self, start: u64, end: u64) -> std::io::Result<Vec<TranscriptEntry>> {
        if start >= end {
            return Ok(Vec::new());
        }
        let mut file = tokio::fs::File::open(&self.path).await?;
        file.seek(std::io::SeekFrom::Start(start)).await?;
        let mut buf = Vec::with_capacity((end - start) as usize);
        let n = file.take(end - start).read_to_end(&mut buf).await?;
        buf.truncate(n);
        let content = String::from_utf8_lossy(&buf);
        let entries = content
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() {
                    return None;
                }
                match serde_json::from_str::<TranscriptEntry>(line) {
                    Ok(entry) => Some(entry),
                    Err(e) => {
                        warn!(error = %e, "failed to parse transcript entry");
                        None
                    }
                }
            })
            .collect();
        Ok(entries)
    }

    /// Slow path: read the whole file, locate the last marker, return entries
    /// after it, and rebuild the sidecar so the next read is cheap.
    async fn full_scan_and_rebuild_sidecar(&self, log_stale: bool) -> (Vec<TranscriptEntry>, bool) {
        if log_stale {
            warn!(path = %self.path.display(), "transcript: sidecar stale, rebuilding via full scan");
        }
        let content = match fs::read_to_string(&self.path).await {
            Ok(c) => c,
            Err(_) => return (Vec::new(), false),
        };

        // Scan line-by-line tracking byte offsets so we can rebuild the sidecar.
        //
        // `byte_offset` follows the same rules as the live sidecar:
        // - On `Extracted` / `Skipped`: advance past the marker (those turns
        //   are processed and should never be re-read).
        // - On `Failed`: leave `byte_offset` where it was (turns remain
        //   unprocessed); recover `fail_attempt` from the marker content.
        let mut cursor: u64 = 0;
        let mut last_marker_end: u64 = 0;
        let mut last_marker_kind: Option<MarkerKind> = None;
        let mut last_marker_unix: i64 = 0;
        let mut fail_attempt: u32 = 0;
        let mut entries_after: Vec<TranscriptEntry> = Vec::new();
        let mut user_turns_since: u32 = 0;

        for raw_line in content.split_inclusive('\n') {
            let line_bytes = raw_line.len() as u64;
            let line = raw_line.trim_matches(|c: char| c == '\n' || c == '\r').trim();
            if !line.is_empty() {
                if let Ok(entry) = serde_json::from_str::<TranscriptEntry>(line) {
                    if entry.role == "system" {
                        if let Some(kind) = MarkerKind::from_content(&entry.content) {
                            let marker_unix = chrono::DateTime::parse_from_rfc3339(&entry.timestamp)
                                .map(|dt| dt.timestamp())
                                .unwrap_or(0);
                            last_marker_kind = Some(kind);
                            last_marker_unix = marker_unix;
                            match kind {
                                MarkerKind::Extracted | MarkerKind::Skipped => {
                                    // Processed: advance past this marker.
                                    last_marker_end = cursor + line_bytes;
                                    entries_after.clear();
                                    user_turns_since = 0;
                                    fail_attempt = 0;
                                }
                                MarkerKind::Failed => {
                                    // Unprocessed: don't advance offset; recover
                                    // attempt count from marker content, fall back
                                    // to monotonic increment for legacy markers
                                    // that didn't encode the attempt.
                                    fail_attempt = parse_diary_failed_attempt(&entry.content)
                                        .unwrap_or_else(|| fail_attempt.saturating_add(1));
                                }
                            }
                            cursor += line_bytes;
                            continue;
                        }
                    }
                    if entry.role == "user" {
                        user_turns_since = user_turns_since.saturating_add(1);
                    }
                    entries_after.push(entry);
                }
            }
            cursor += line_bytes;
        }

        let state = MarkerState {
            byte_offset: last_marker_end,
            user_turns_since,
            last_marker_kind,
            last_marker_unix,
            fail_attempt,
        };
        self.write_marker_state(&state).await;
        (entries_after, last_marker_kind.is_some())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append a single entry to the JSONL file. Returns the file size after
    /// the write so callers (marker writes) can persist that offset to the
    /// sidecar.
    async fn append(&self, entry: &TranscriptEntry) -> Option<u64> {
        let json = match serde_json::to_string(entry) {
            Ok(j) => j,
            Err(e) => {
                warn!(error = %e, "failed to serialize transcript entry");
                return None;
            }
        };

        let result = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await;

        match result {
            Ok(mut file) => {
                let line = format!("{}\n", json);
                if let Err(e) = file.write_all(line.as_bytes()).await {
                    warn!(error = %e, "failed to write transcript entry");
                    return None;
                }
                // Flush so the on-disk size matches what we report. Append-mode
                // writes are atomic per syscall but the file metadata may lag.
                let _ = file.flush().await;
                fs::metadata(&self.path).await.ok().map(|m| m.len())
            }
            Err(e) => {
                warn!(error = %e, path = %self.path.display(), "failed to open transcript file");
                None
            }
        }
    }
}

/// Extract the attempt number from a `diary_failed:{n}:{rfc3339}` marker
/// content string. Returns `None` for legacy `diary_failed:{rfc3339}` markers
/// (no attempt encoded) so the caller can fall back to its own counter.
pub fn parse_diary_failed_attempt(content: &str) -> Option<u32> {
    let rest = content.strip_prefix("diary_failed:")?;
    // New format: "{n}:{rfc3339}". The attempt is the segment up to the
    // first `:`. Older markers wrote just "{rfc3339}" (digits + `-` + `T`
    // + ...), where the leading segment before the first `:` is the year
    // and won't be a small attempt count. Discriminate by length + range.
    let (head, tail) = rest.split_once(':')?;
    // Year segments are 4 digits ≥ 1000; attempt counts will be small
    // (cap is 4 in DIARY_FAILURE_BACKOFF_SECS table) and the tail must
    // look like an RFC3339 timestamp (starts with a 4-digit year).
    if !tail.chars().take(4).all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: u32 = head.parse().ok()?;
    if n > 1000 {
        // Looks like a year, not an attempt count — legacy format.
        return None;
    }
    Some(n)
}

/// Drop `diary_failed:*` system rows from a slice of entries. Used by
/// `read_since_last_marker` so retries after a Failed marker don't feed the
/// audit-trail line back into the LLM prompt as if it were content.
fn filter_diary_failed_markers(entries: Vec<TranscriptEntry>) -> Vec<TranscriptEntry> {
    entries
        .into_iter()
        .filter(|e| !(e.role == "system" && e.content.starts_with("diary_failed")))
        .collect()
}

/// Build a transcript label from a session key.
/// e.g. "main:discord:dm.Boze" → "discord_dm.Boze"
/// Extracts origin and context_id, dropping the agent_id prefix.
pub fn label_from_session_key(session_key: &str) -> String {
    let parts: Vec<&str> = session_key.splitn(3, ':').collect();
    if parts.len() >= 3 {
        format!("{}_{}", parts[1], parts[2])
    } else {
        session_key.to_string()
    }
}

/// Sanitize a label for use in filenames: keep alphanumeric, dot, dash, underscore.
fn sanitize_label(label: &str) -> String {
    label
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
