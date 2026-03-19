use std::path::PathBuf;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;

use super::theme::Theme;
use super::{Action, Component};
use crate::logging;

const AUTO_REFRESH_INTERVAL_SECS: u64 = 2;

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Normal,
    Search,
}

pub struct LogsPanel {
    records: Vec<logging::LogRecord>,
    level_filter: LogLevel,
    scroll: u16,
    log_dir: Option<PathBuf>,
    last_refresh: Instant,
    mode: Mode,
    search_buf: String,
    search_query: Option<String>,
    /// Total rendered lines (updated each render, used to clamp scroll)
    total_lines: u16,
    /// Visible content height in lines (updated each render)
    visible_height: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    fn color(&self) -> Color {
        match self {
            LogLevel::Error => Theme::RED,
            LogLevel::Warn => Theme::YELLOW,
            LogLevel::Info => Theme::GREEN,
            LogLevel::Debug => Theme::OVERLAY1,
        }
    }

    fn label(&self) -> &str {
        match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warn => "WARN ",
            LogLevel::Info => "INFO ",
            LogLevel::Debug => "DEBUG",
        }
    }

    fn matches(&self, level_str: &str) -> bool {
        let priority = match self {
            LogLevel::Error => 4,
            LogLevel::Warn => 3,
            LogLevel::Info => 2,
            LogLevel::Debug => 1,
        };
        let record_priority = match level_str.to_uppercase().as_str() {
            "ERROR" => 4,
            "WARN" => 3,
            "INFO" => 2,
            "DEBUG" | "TRACE" => 1,
            _ => 2,
        };
        record_priority >= priority
    }

    fn from_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "ERROR" => LogLevel::Error,
            "WARN" => LogLevel::Warn,
            "DEBUG" | "TRACE" => LogLevel::Debug,
            _ => LogLevel::Info,
        }
    }
}

impl LogsPanel {
    pub fn new(log_dir: Option<PathBuf>, initial_level: &str) -> Self {
        let mut panel = LogsPanel {
            records: Vec::new(),
            level_filter: LogLevel::from_str(initial_level),
            scroll: 0,
            log_dir,
            last_refresh: Instant::now(),
            mode: Mode::Normal,
            search_buf: String::new(),
            search_query: None,
            total_lines: 0,
            visible_height: 0,
        };
        panel.refresh();
        panel
    }

    pub fn set_level(&mut self, level: &str) {
        self.level_filter = LogLevel::from_str(level);
    }

    pub fn add_log(&mut self, level: LogLevel, message: String) {
        self.records.push(logging::LogRecord {
            ts: chrono::Utc::now().to_rfc3339(),
            level: level.label().trim().to_string(),
            target: "catclaw::tui".to_string(),
            msg: message,
            fields: serde_json::Map::new(),
        });
    }

    pub fn refresh(&mut self) {
        self.last_refresh = Instant::now();

        let Some(log_dir) = &self.log_dir else {
            return;
        };

        // Read today's log file (and optionally yesterday's for recent context)
        let today = logging::today_log_path(log_dir);
        let mut records = logging::read_log_file(&today);

        // If today's file is small, also load yesterday's
        if records.len() < 50 {
            let yesterday = chrono::Local::now()
                .date_naive()
                .pred_opt()
                .map(|d| log_dir.join(format!("catclaw-{}.jsonl", d)));
            if let Some(path) = yesterday {
                let mut old = logging::read_log_file(&path);
                old.append(&mut records);
                records = old;
            }
        }

        // Also try reading the legacy gateway.log (tracing-subscriber text format)
        // for backwards compatibility during transition
        if records.is_empty() {
            let legacy_path = log_dir
                .parent()
                .map(|p| p.join("gateway.log"))
                .unwrap_or_default();
            if legacy_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&legacy_path) {
                    for line in content.lines() {
                        if line.trim().is_empty() {
                            continue;
                        }
                        records.push(parse_legacy_log_line(line));
                    }
                }
            }
        }

        self.records = records;
    }

    fn filtered_records(&self) -> Vec<(usize, &logging::LogRecord)> {
        self.records
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, r)| self.level_filter.matches(&r.level))
            .filter(|(_, r)| {
                if let Some(ref query) = self.search_query {
                    let q = query.to_lowercase();
                    r.msg.to_lowercase().contains(&q)
                        || r.target.to_lowercase().contains(&q)
                } else {
                    true
                }
            })
            .collect()
    }

    pub fn scroll_up(&mut self, lines: u16) {
        self.scroll = self.scroll.saturating_sub(lines);
    }

    pub fn scroll_down(&mut self, lines: u16) {
        let max = self.total_lines.saturating_sub(self.visible_height);
        self.scroll = self.scroll.saturating_add(lines).min(max);
    }

    fn scroll_to_bottom(&mut self) {
        let count = self.filtered_records().len();
        if count > 0 {
            self.scroll = count.saturating_sub(1) as u16;
        }
    }
}

/// Parse a legacy tracing-subscriber text line into a LogRecord.
fn parse_legacy_log_line(line: &str) -> logging::LogRecord {
    // Format: "2026-03-10T12:34:56.789Z  INFO catclaw::gateway: message"
    let mut parts = line.splitn(2, char::is_whitespace);
    let timestamp_raw = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim_start();

    let mut rest_parts = rest.splitn(2, char::is_whitespace);
    let level_str = rest_parts.next().unwrap_or("");
    let message_rest = rest_parts.next().unwrap_or("").trim_start();

    let level = match level_str {
        "ERROR" => "ERROR",
        "WARN" => "WARN",
        "DEBUG" | "TRACE" => "DEBUG",
        "INFO" => "INFO",
        _ => {
            return logging::LogRecord {
                ts: timestamp_raw.to_string(),
                level: "INFO".to_string(),
                target: String::new(),
                msg: line.to_string(),
                fields: serde_json::Map::new(),
            };
        }
    };

    // Try to split target from message: "catclaw::gateway: actual message"
    let (target, msg) = if let Some(colon_pos) = message_rest.find(": ") {
        (
            message_rest[..colon_pos].to_string(),
            message_rest[colon_pos + 2..].to_string(),
        )
    } else {
        (String::new(), message_rest.to_string())
    };

    logging::LogRecord {
        ts: timestamp_raw.to_string(),
        level: level.to_string(),
        target,
        msg,
        fields: serde_json::Map::new(),
    }
}

impl Component for LogsPanel {
    fn handle_event(&mut self, event: &KeyEvent) -> Action {
        match &self.mode {
            Mode::Search => match event.code {
                KeyCode::Enter => {
                    let query = self.search_buf.trim().to_string();
                    self.search_query = if query.is_empty() {
                        None
                    } else {
                        Some(query)
                    };
                    self.mode = Mode::Normal;
                    self.scroll = 0;
                    Action::None
                }
                KeyCode::Esc => {
                    self.mode = Mode::Normal;
                    Action::None
                }
                KeyCode::Backspace => {
                    self.search_buf.pop();
                    Action::None
                }
                KeyCode::Char(c) => {
                    self.search_buf.push(c);
                    Action::None
                }
                _ => Action::None,
            },
            Mode::Normal => match event.code {
                KeyCode::Char('1') => {
                    self.level_filter = LogLevel::Error;
                    self.scroll = 0;
                    Action::None
                }
                KeyCode::Char('2') => {
                    self.level_filter = LogLevel::Warn;
                    self.scroll = 0;
                    Action::None
                }
                KeyCode::Char('3') => {
                    self.level_filter = LogLevel::Info;
                    self.scroll = 0;
                    Action::None
                }
                KeyCode::Char('4') => {
                    self.level_filter = LogLevel::Debug;
                    self.scroll = 0;
                    Action::None
                }
                KeyCode::Char('/') => {
                    self.search_buf.clear();
                    self.mode = Mode::Search;
                    Action::None
                }
                KeyCode::Char('c') => {
                    // Clear search — jump to newest (top)
                    self.search_query = None;
                    self.scroll = 0;
                    Action::None
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    let max = self.total_lines.saturating_sub(self.visible_height);
                    self.scroll = self.scroll.saturating_add(1).min(max);
                    Action::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.scroll = self.scroll.saturating_sub(1);
                    Action::None
                }
                KeyCode::Char('G') => {
                    self.scroll_to_bottom();
                    Action::None
                }
                KeyCode::Char('g') => {
                    self.scroll = 0;
                    Action::None
                }
                KeyCode::Char('r') => {
                    self.refresh();
                    self.scroll = 0;
                    Action::None
                }
                _ => Action::None,
            },
        }
    }

    fn captures_input(&self) -> bool {
        self.mode == Mode::Search
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Auto-refresh
        if self.last_refresh.elapsed().as_secs() >= AUTO_REFRESH_INTERVAL_SECS {
            self.refresh();
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),    // Log content
                Constraint::Length(1), // Status / search input
                Constraint::Length(1), // Help
            ])
            .split(area);

        let filtered = self.filtered_records();
        let search_query = self.search_query.clone();

        // Calculate available width for message text (after prefix)
        // Prefix: " HH:MM:SS " (10) + "LEVEL " (6) + "[target] " (variable)
        let panel_inner_width = chunks[0].width.saturating_sub(2) as usize; // subtract borders

        let lines: Vec<Line> = filtered
            .iter()
            .flat_map(|(_, record)| {
                let time = extract_time(&record.ts);
                let level = LogLevel::from_str(&record.level);

                let target_short = if let Some(stripped) = record.target.strip_prefix("catclaw::") {
                    format!("[{}] ", stripped)
                } else if record.target.is_empty() {
                    String::new()
                } else {
                    format!("[{}] ", record.target)
                };

                // Build extra fields string (truncate long values to keep display manageable)
                let extra: String = if record.fields.is_empty() {
                    String::new()
                } else {
                    let pairs: Vec<String> = record
                        .fields
                        .iter()
                        .map(|(k, v)| {
                            let vs = match v {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            if vs.chars().count() > 120 {
                                let truncated: String = vs.chars().take(120).collect();
                                format!(" {}={}...", k, truncated)
                            } else {
                                format!(" {}={}", k, vs)
                            }
                        })
                        .collect();
                    pairs.join("")
                };

                let msg_with_extra = format!("{}{}", record.msg, extra);

                // Calculate prefix width: " HH:MM:SS " + "LEVEL " + "[target] "
                let prefix_width = 1 + time.len() + 1 + level.label().len() + 1 + target_short.len();
                let msg_max_width = panel_inner_width.saturating_sub(prefix_width);

                if msg_max_width == 0 || msg_with_extra.chars().count() <= msg_max_width {
                    // Fits in one line
                    let msg_spans = if let Some(ref query) = search_query {
                        highlight_matches(&msg_with_extra, query)
                    } else {
                        vec![Span::styled(msg_with_extra, Style::default().fg(Theme::TEXT))]
                    };

                    let mut spans = vec![
                        Span::styled(
                            format!(" {} ", time),
                            Style::default().fg(Theme::OVERLAY0),
                        ),
                        Span::styled(
                            format!("{} ", level.label()),
                            Style::default().fg(level.color()),
                        ),
                        Span::styled(target_short, Style::default().fg(Theme::SURFACE2)),
                    ];
                    spans.extend(msg_spans);
                    vec![Line::from(spans)]
                } else {
                    // Wrap into multiple lines
                    let wrapped = wrap_log_text(&msg_with_extra, msg_max_width);
                    let indent = " ".repeat(prefix_width);
                    let mut result_lines = Vec::new();
                    for (i, chunk) in wrapped.iter().enumerate() {
                        if i == 0 {
                            let msg_spans = if let Some(ref query) = search_query {
                                highlight_matches(chunk, query)
                            } else {
                                vec![Span::styled(chunk.to_string(), Style::default().fg(Theme::TEXT))]
                            };
                            let mut spans = vec![
                                Span::styled(
                                    format!(" {} ", time),
                                    Style::default().fg(Theme::OVERLAY0),
                                ),
                                Span::styled(
                                    format!("{} ", level.label()),
                                    Style::default().fg(level.color()),
                                ),
                                Span::styled(target_short.clone(), Style::default().fg(Theme::SURFACE2)),
                            ];
                            spans.extend(msg_spans);
                            result_lines.push(Line::from(spans));
                        } else {
                            let msg_spans = if let Some(ref query) = search_query {
                                highlight_matches(chunk, query)
                            } else {
                                vec![Span::styled(chunk.to_string(), Style::default().fg(Theme::TEXT))]
                            };
                            let mut spans = vec![
                                Span::styled(indent.clone(), Style::default()),
                            ];
                            spans.extend(msg_spans);
                            result_lines.push(Line::from(spans));
                        }
                    }
                    result_lines
                }
            })
            .collect();

        let total = lines.len();
        // Store for scroll clamping in handle_event
        self.total_lines = total as u16;
        self.visible_height = chunks[0].height.saturating_sub(2); // subtract borders
        // Clamp scroll so we never go past the last line
        let max_scroll = self.total_lines.saturating_sub(self.visible_height);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Theme::SURFACE1))
                    .title(" Logs ")
                    .title_style(Style::default().fg(Theme::MAUVE)),
            )
            .scroll((self.scroll, 0));

        frame.render_widget(paragraph, chunks[0]);

        // Status / search input line
        let status_line = match &self.mode {
            Mode::Search => {
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        " /",
                        Style::default()
                            .fg(Theme::MAUVE)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{}▌", self.search_buf),
                        Style::default().fg(Theme::TEXT),
                    ),
                ]))
                .style(Style::default().bg(Theme::SURFACE0))
            }
            Mode::Normal => {
                let level_label = match self.level_filter {
                    LogLevel::Error => "ERROR",
                    LogLevel::Warn => "WARN+",
                    LogLevel::Info => "INFO+",
                    LogLevel::Debug => "ALL",
                };
                let search_info = if let Some(ref q) = self.search_query {
                    format!("  search: \"{}\"", q)
                } else {
                    String::new()
                };
                Paragraph::new(format!(
                    " [{}]  {} entries{}",
                    level_label, total, search_info
                ))
                .style(Style::default().fg(Theme::GREEN).bg(Theme::MANTLE))
            }
        };
        frame.render_widget(status_line, chunks[1]);

        // Help bar
        let help_text = match &self.mode {
            Mode::Search => " Enter Search  Esc Cancel",
            Mode::Normal => {
                if self.search_query.is_some() {
                    " 1-4 Level  / Search  c Clear search  g/G Top/Bottom  j/k Scroll  r Refresh"
                } else {
                    " 1-4 Level  / Search  g/G Top/Bottom  j/k Scroll  r Refresh"
                }
            }
        };
        let help = Paragraph::new(help_text)
            .style(Style::default().fg(Theme::OVERLAY0).bg(Theme::MANTLE));
        frame.render_widget(help, chunks[2]);
    }
}

/// Wrap a log message into chunks of max_width characters.
/// Breaks at word boundaries when possible, otherwise hard-wraps.
/// Uses char boundaries to be safe with multi-byte UTF-8.
fn wrap_log_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 || text.chars().count() <= max_width {
        return vec![text.to_string()];
    }

    let mut result = Vec::new();
    let mut remaining = text;
    while remaining.chars().count() > max_width {
        // Find the byte offset of the max_width-th character
        let byte_end = remaining
            .char_indices()
            .nth(max_width)
            .map(|(i, _)| i)
            .unwrap_or(remaining.len());
        // Try to find a word boundary within the first max_width chars
        let break_at = remaining[..byte_end]
            .rfind(|c: char| c.is_whitespace() || c == '=' || c == ',' || c == '/')
            .map(|pos| pos + 1)
            .unwrap_or(byte_end); // hard-wrap if no good break point
        result.push(remaining[..break_at].to_string());
        remaining = &remaining[break_at..];
    }
    if !remaining.is_empty() {
        result.push(remaining.to_string());
    }
    result
}

fn extract_time(ts: &str) -> String {
    if let Some(t_pos) = ts.find('T') {
        let after = &ts[t_pos + 1..];
        if after.len() >= 8 {
            after[..8].to_string()
        } else {
            after.trim_end_matches('Z').to_string()
        }
    } else {
        ts.to_string()
    }
}

/// Highlight occurrences of `query` in `text` with a different style.
/// Returns owned Span<'static> to avoid lifetime issues.
fn highlight_matches(text: &str, query: &str) -> Vec<Span<'static>> {
    if query.is_empty() {
        return vec![Span::styled(
            text.to_string(),
            Style::default().fg(Theme::TEXT),
        )];
    }

    let query_lower = query.to_lowercase();
    let text_lower = text.to_lowercase();
    let mut spans = Vec::new();
    let mut last_end = 0;

    for (start, _) in text_lower.match_indices(&query_lower) {
        let end = start + query.len();
        if start > last_end {
            spans.push(Span::styled(
                text[last_end..start].to_string(),
                Style::default().fg(Theme::TEXT),
            ));
        }
        spans.push(Span::styled(
            text[start..end].to_string(),
            Style::default()
                .fg(Theme::CRUST)
                .bg(Theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        ));
        last_end = end;
    }

    if last_end < text.len() {
        spans.push(Span::styled(
            text[last_end..].to_string(),
            Style::default().fg(Theme::TEXT),
        ));
    }

    if spans.is_empty() {
        spans.push(Span::styled(
            text.to_string(),
            Style::default().fg(Theme::TEXT),
        ));
    }

    spans
}
