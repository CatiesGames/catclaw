use std::path::PathBuf;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;
use tokio::sync::mpsc;

use super::theme::Theme;
use super::{Action, Component};
use crate::config::{BindingConfig, Config};
use crate::ws_client::GatewayClient;

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    /// Browsing bindings list
    Normal,
    /// Adding a new binding — entering pattern
    AddPattern,
    /// Adding/editing — entering agent (with autocomplete)
    InputAgent,
    /// Confirm delete
    ConfirmDelete,
}

pub struct BindingsPanel {
    bindings: Vec<BindingConfig>,
    selected: usize,
    mode: Mode,
    /// Text input buffer for pattern
    pattern_buf: String,
    /// Text input buffer for agent
    agent_buf: String,
    /// Known agent IDs for autocomplete
    agent_ids: Vec<String>,
    /// Currently highlighted autocomplete suggestion index
    autocomplete_idx: usize,
    /// Status message
    status_msg: Option<String>,
    /// WS client — gateway is the sole writer of catclaw.toml + router state
    client: Arc<GatewayClient>,
    /// Reports failures from background WS calls so the user notices when a
    /// "saved" message was actually lost in transit.
    op_err_tx: mpsc::UnboundedSender<String>,
    op_err_rx: mpsc::UnboundedReceiver<String>,
}

impl BindingsPanel {
    /// `_config_path` is unused (the gateway is the sole writer now), but kept
    /// in the signature so the construction site looks identical to its
    /// siblings (agents/config panels).
    pub fn new(config: &Config, _config_path: PathBuf, client: Arc<GatewayClient>) -> Self {
        let agent_ids: Vec<String> = config.agents.iter().map(|a| a.id.clone()).collect();
        let (op_err_tx, op_err_rx) = mpsc::unbounded_channel();

        BindingsPanel {
            bindings: config.bindings.clone(),
            selected: 0,
            mode: Mode::Normal,
            pattern_buf: String::new(),
            agent_buf: String::new(),
            agent_ids,
            autocomplete_idx: 0,
            status_msg: None,
            client,
            op_err_tx,
            op_err_rx,
        }
    }

    /// Drain background WS errors and surface the latest one.
    fn poll_op_errors(&mut self) {
        while let Ok(msg) = self.op_err_rx.try_recv() {
            self.status_msg = Some(msg);
        }
    }

    /// Get autocomplete suggestions filtered by current agent_buf input
    fn filtered_agents(&self) -> Vec<&str> {
        let query = self.agent_buf.to_lowercase();
        if query.is_empty() {
            self.agent_ids.iter().map(|s| s.as_str()).collect()
        } else {
            self.agent_ids
                .iter()
                .filter(|id| id.to_lowercase().contains(&query))
                .map(|s| s.as_str())
                .collect()
        }
    }

    /// Accept the current autocomplete selection
    fn accept_autocomplete(&mut self) {
        let filtered = self.filtered_agents();
        if let Some(&agent) = filtered.get(self.autocomplete_idx) {
            self.agent_buf = agent.to_string();
        }
    }

    fn save_binding(&mut self) {
        let pattern = self.pattern_buf.trim().to_string();
        let agent = self.agent_buf.trim().to_string();

        if pattern.is_empty() || agent.is_empty() {
            self.status_msg = Some("❌ Pattern and agent cannot be empty".to_string());
            self.mode = Mode::Normal;
            return;
        }

        // Optimistic local update; gateway is authoritative.
        self.bindings.retain(|b| b.pattern != pattern);
        self.bindings.push(BindingConfig {
            pattern: pattern.clone(),
            agent: agent.clone(),
        });
        self.status_msg = Some(format!("✅ Bound '{}' → '{}'", pattern, agent));

        let client = self.client.clone();
        let err_tx = self.op_err_tx.clone();
        let p = pattern.clone();
        let a = agent.clone();
        tokio::spawn(async move {
            if let Err(e) = client.request(
                "bindings.set",
                serde_json::json!({"pattern": &p, "agent": &a}),
            ).await {
                let _ = err_tx.send(format!("Failed to save binding '{}': {}", p, e));
            }
        });

        self.mode = Mode::Normal;
        self.pattern_buf.clear();
        self.agent_buf.clear();
    }

    fn delete_selected(&mut self) {
        if let Some(entry) = self.bindings.get(self.selected) {
            let pattern = entry.pattern.clone();
            self.bindings.retain(|b| b.pattern != pattern);
            if self.selected >= self.bindings.len() && !self.bindings.is_empty() {
                self.selected = self.bindings.len() - 1;
            }
            self.status_msg = Some(format!("🗑️ Deleted binding '{}'", pattern));

            let client = self.client.clone();
            let err_tx = self.op_err_tx.clone();
            let p = pattern.clone();
            tokio::spawn(async move {
                if let Err(e) = client.request(
                    "bindings.delete",
                    serde_json::json!({"pattern": &p}),
                ).await {
                    let _ = err_tx.send(format!("Failed to delete binding '{}': {}", p, e));
                }
            });
        }
        self.mode = Mode::Normal;
    }
}

impl Component for BindingsPanel {
    fn handle_event(&mut self, event: &KeyEvent) -> Action {
        match &self.mode {
            Mode::Normal => match event.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    if !self.bindings.is_empty() {
                        self.selected = (self.selected + 1).min(self.bindings.len() - 1);
                    }
                    Action::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    Action::None
                }
                KeyCode::Char('a') => {
                    self.pattern_buf.clear();
                    self.agent_buf.clear();
                    self.autocomplete_idx = 0;
                    self.mode = Mode::AddPattern;
                    self.status_msg = None;
                    Action::None
                }
                KeyCode::Char('e') => {
                    if let Some(entry) = self.bindings.get(self.selected) {
                        self.pattern_buf = entry.pattern.clone();
                        self.agent_buf = entry.agent.clone();
                        self.autocomplete_idx = 0;
                        self.mode = Mode::InputAgent;
                        self.status_msg = None;
                    }
                    Action::None
                }
                KeyCode::Char('d') => {
                    if !self.bindings.is_empty() {
                        self.mode = Mode::ConfirmDelete;
                        self.status_msg = None;
                    }
                    Action::None
                }
                _ => Action::None,
            },

            Mode::AddPattern => match event.code {
                KeyCode::Enter => {
                    if !self.pattern_buf.trim().is_empty() {
                        self.autocomplete_idx = 0;
                        self.mode = Mode::InputAgent;
                    }
                    Action::None
                }
                KeyCode::Esc => {
                    self.mode = Mode::Normal;
                    self.status_msg = Some("Cancelled".to_string());
                    Action::None
                }
                KeyCode::Backspace => {
                    self.pattern_buf.pop();
                    Action::None
                }
                KeyCode::Char(c) => {
                    self.pattern_buf.push(c);
                    Action::None
                }
                _ => Action::None,
            },

            Mode::InputAgent => match event.code {
                KeyCode::Enter => {
                    if self.agent_buf.is_empty() {
                        self.accept_autocomplete();
                    }
                    self.save_binding();
                    Action::None
                }
                KeyCode::Tab => {
                    self.accept_autocomplete();
                    Action::None
                }
                KeyCode::Down => {
                    let count = self.filtered_agents().len();
                    if count > 0 {
                        self.autocomplete_idx = (self.autocomplete_idx + 1).min(count - 1);
                    }
                    Action::None
                }
                KeyCode::Up => {
                    self.autocomplete_idx = self.autocomplete_idx.saturating_sub(1);
                    Action::None
                }
                KeyCode::Esc => {
                    self.mode = Mode::Normal;
                    self.status_msg = Some("Cancelled".to_string());
                    Action::None
                }
                KeyCode::Backspace => {
                    self.agent_buf.pop();
                    self.autocomplete_idx = 0;
                    Action::None
                }
                KeyCode::Char(c) => {
                    self.agent_buf.push(c);
                    self.autocomplete_idx = 0;
                    Action::None
                }
                _ => Action::None,
            },

            Mode::ConfirmDelete => match event.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.delete_selected();
                    Action::None
                }
                _ => {
                    self.mode = Mode::Normal;
                    self.status_msg = Some("Cancelled".to_string());
                    Action::None
                }
            },
        }
    }

    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Normal)
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.poll_op_errors();
        let status_height = if matches!(self.mode, Mode::AddPattern) { 2 } else { 1 };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(status_height), // Status / input
                Constraint::Length(1),             // Help
            ])
            .split(area);

        // ── Table ──
        let header = Row::new(vec!["  Pattern", "Agent"])
            .style(
                Style::default()
                    .fg(Theme::MAUVE)
                    .add_modifier(Modifier::BOLD),
            )
            .bottom_margin(1);

        let rows: Vec<Row> = self
            .bindings
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let is_selected = i == self.selected;
                let style = if is_selected {
                    Style::default().fg(Theme::TEXT).bg(Theme::SURFACE0)
                } else {
                    Style::default().fg(Theme::SUBTEXT0)
                };

                Row::new(vec![
                    Cell::from(format!("  {}", b.pattern)).style(style),
                    Cell::from(b.agent.clone()).style(if is_selected {
                        Style::default()
                            .fg(Theme::MAUVE)
                            .bg(Theme::SURFACE0)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Theme::LAVENDER)
                    }),
                ])
            })
            .collect();

        // Adjust table area if in agent input mode (need space for autocomplete)
        let (table_area, autocomplete_area) =
            if matches!(self.mode, Mode::InputAgent) {
                let filtered_count = self.filtered_agents().len().min(5) as u16;
                if filtered_count > 0 && chunks[0].height > filtered_count + 4 {
                    let split = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Min(0),
                            Constraint::Length(filtered_count + 2),
                        ])
                        .split(chunks[0]);
                    (split[0], Some(split[1]))
                } else {
                    (chunks[0], None)
                }
            } else {
                (chunks[0], None)
            };

        let empty_msg = if self.bindings.is_empty() {
            "  No bindings. All channels use the default agent. Press 'a' to add."
        } else {
            ""
        };

        let table = Table::new(
            rows,
            [Constraint::Percentage(60), Constraint::Percentage(40)],
        )
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Theme::SURFACE1))
                .title(" 🔗 Bindings ")
                .title_style(Style::default().fg(Theme::MAUVE)),
        );

        frame.render_widget(table, table_area);

        // Show empty message if no bindings
        if self.bindings.is_empty() && matches!(self.mode, Mode::Normal) {
            let inner = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .inner(table_area);
            let msg = Paragraph::new(vec![
                Line::from(""),
                Line::from(""),
                Line::from(Span::styled(
                    empty_msg,
                    Style::default().fg(Theme::OVERLAY0),
                )),
            ]);
            frame.render_widget(msg, inner);
        }

        // ── Autocomplete popup ──
        if let Some(ac_area) = autocomplete_area {
            self.render_autocomplete(frame, ac_area);
        }

        // ── Status / Input line ──
        let status_line = match &self.mode {
            Mode::AddPattern => {
                // Build pattern anatomy hint based on what's typed so far
                let hint = pattern_hint(&self.pattern_buf);
                Paragraph::new(vec![
                    Line::from(vec![
                        Span::styled(" Pattern: ", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("{}▌", self.pattern_buf), Style::default().fg(Theme::TEXT)),
                    ]),
                    Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(hint, Style::default().fg(Theme::OVERLAY1)),
                    ]),
                ])
                .style(Style::default().bg(Theme::SURFACE0))
            }

            Mode::InputAgent => {
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!(" Agent for '{}': ", self.pattern_buf),
                        Style::default()
                            .fg(Theme::MAUVE)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{}▌", self.agent_buf),
                        Style::default().fg(Theme::TEXT),
                    ),
                    Span::styled(
                        "  Tab complete  ↑↓ select",
                        Style::default().fg(Theme::SURFACE2),
                    ),
                ]))
                .style(Style::default().bg(Theme::SURFACE0))
            }

            Mode::ConfirmDelete => {
                let name = self
                    .bindings
                    .get(self.selected)
                    .map(|b| b.pattern.as_str())
                    .unwrap_or("?");
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!(" 🗑️ Delete binding '{}'? ", name),
                        Style::default()
                            .fg(Theme::RED)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("(y/N)", Style::default().fg(Theme::OVERLAY0)),
                ]))
                .style(Style::default().bg(Theme::SURFACE0))
            }

            Mode::Normal => {
                if let Some(msg) = &self.status_msg {
                    Paragraph::new(format!(" {}", msg))
                        .style(Style::default().fg(Theme::GREEN).bg(Theme::MANTLE))
                } else {
                    Paragraph::new("").style(Style::default().bg(Theme::MANTLE))
                }
            }
        };
        frame.render_widget(status_line, chunks[1]);

        // ── Help bar ──
        let help_text = match &self.mode {
            Mode::AddPattern => " Enter Next  Esc Cancel",
            Mode::InputAgent => " Enter Save  Tab Complete  ↑↓ Select  Esc Cancel",
            Mode::ConfirmDelete => " y Delete  any other key Cancel",
            Mode::Normal => " a Add  e Edit  d Delete",
        };
        let help = Paragraph::new(help_text)
            .style(Style::default().fg(Theme::OVERLAY0).bg(Theme::MANTLE));
        frame.render_widget(help, chunks[2]);
    }
}

impl BindingsPanel {
    fn render_autocomplete(&self, frame: &mut Frame, area: Rect) {
        let filtered = self.filtered_agents();
        if filtered.is_empty() {
            let msg = Paragraph::new("  No matching agents")
                .style(Style::default().fg(Theme::OVERLAY0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(Theme::SURFACE1))
                        .title(" 🤖 Agents ")
                        .title_style(Style::default().fg(Theme::OVERLAY0)),
                );
            frame.render_widget(msg, area);
            return;
        }

        let items: Vec<ListItem> = filtered
            .iter()
            .enumerate()
            .take(5)
            .map(|(i, &agent)| {
                let is_selected = i == self.autocomplete_idx;
                let (prefix, style) = if is_selected {
                    (
                        "  ▸ ",
                        Style::default()
                            .fg(Theme::MAUVE)
                            .bg(Theme::SURFACE0)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("    ", Style::default().fg(Theme::SUBTEXT0))
                };
                ListItem::new(Line::from(vec![
                    Span::styled(prefix, style),
                    Span::styled(agent, style),
                ]))
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Theme::MAUVE))
                .title(" 🤖 Agents ")
                .title_style(Style::default().fg(Theme::MAUVE)),
        );

        frame.render_widget(list, area);
    }
}

/// Generate a contextual hint explaining the pattern structure based on what's typed.
fn pattern_hint(buf: &str) -> String {
    let parts: Vec<&str> = buf.splitn(4, ':').collect();
    match parts.as_slice() {
        [] | [""] => {
            "Format: {platform}:{scope}:{id}  e.g. discord:channel:123  |  telegram:*  |  *".to_string()
        }
        [platform] => {
            let scopes = match *platform {
                "discord"  => "dm | channel | guild | *",
                "telegram" => "dm | *",
                "*"        => "(global wildcard — matches all platforms)",
                _          => "dm | channel | guild | *",
            };
            format!("{platform}:  ← next: scope [{scopes}]")
        }
        [platform, scope] => {
            let id_hint = match (*platform, *scope) {
                (_, "dm")      => "user ID  e.g. 123456789",
                (_, "channel") => "channel ID  e.g. 987654321",
                (_, "guild")   => "guild/server ID  e.g. 111222333",
                (_, "*")       => "(wildcard — matches all in this platform)",
                _              => "ID or *",
            };
            format!("{platform}:{scope}:  ← next: {id_hint}")
        }
        [platform, scope, id, ..] => {
            format!("✓  {platform}:{scope}:{id}  — matches {}", match (*platform, *scope) {
                ("discord",  "dm")      => "Discord DM from this user",
                ("discord",  "channel") => "messages in this Discord channel",
                ("discord",  "guild")   => "all messages in this Discord server",
                ("discord",  "*")       => "all Discord messages",
                ("telegram", "dm")      => "Telegram DM from this user",
                ("telegram", "*")       => "all Telegram messages",
                ("*",        _)         => "all platforms (global fallback)",
                _                       => "messages matching this pattern",
            })
        }
    }
}
