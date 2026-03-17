use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
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

/// Manages transcript files for a session, stored under the agent's workspace.
///
/// Layout: {agent_workspace}/transcripts/{session_id}.jsonl
pub struct TranscriptLog {
    path: PathBuf,
}

#[allow(dead_code)]
impl TranscriptLog {
    /// Create a new transcript log for a session.
    /// Ensures the transcripts/ directory exists.
    pub async fn open(agent_workspace: &Path, session_id: &str) -> Result<Self> {
        let dir = agent_workspace.join("transcripts");
        fs::create_dir_all(&dir).await?;
        let path = dir.join(format!("{}.jsonl", session_id));
        Ok(TranscriptLog { path })
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

    /// Append a user message to the transcript
    pub async fn log_user(
        &self,
        message: &str,
        sender_id: Option<&str>,
        sender_name: Option<&str>,
    ) {
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

    /// Append a system event to the transcript
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
        self.append(&entry).await;
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

    /// Read entries since the last diary marker (`diary_extracted` or `diary_skipped`).
    ///
    /// Returns `(entries_after_marker, marker_was_found)`.
    /// If no marker exists, returns all entries.
    pub async fn read_since_last_marker(&self) -> (Vec<TranscriptEntry>, bool) {
        let all = self.read_all().await;

        // Find the last diary marker (scanning from the end)
        let marker_pos = all.iter().rposition(|e| {
            e.role == "system"
                && (e.content.starts_with("diary_extracted")
                    || e.content.starts_with("diary_skipped"))
        });

        match marker_pos {
            Some(pos) => {
                let entries = all[(pos + 1)..].to_vec();
                (entries, true)
            }
            None => (all, false),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append a single entry to the JSONL file
    async fn append(&self, entry: &TranscriptEntry) {
        let json = match serde_json::to_string(entry) {
            Ok(j) => j,
            Err(e) => {
                warn!(error = %e, "failed to serialize transcript entry");
                return;
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
                }
            }
            Err(e) => {
                warn!(error = %e, path = %self.path.display(), "failed to open transcript file");
            }
        }
    }
}
