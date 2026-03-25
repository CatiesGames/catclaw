use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;
use serde_json::json;
use tokio::sync::mpsc;

use super::theme::Theme;
use super::{Action, Component};
use crate::ws_client::GatewayClient;

#[derive(Debug, Clone)]
struct IssueItem {
    id: String,
    agent_id: String,
    level: String,
    target: String,
    msg: String,
    last_seen: String,
    count: u32,
    status: String,
}

enum IssuesEvent {
    Loaded(Vec<IssueItem>),
    ActionDone(String),
    Error(String),
}

pub struct IssuesPanel {
    client: Arc<GatewayClient>,
    items: Vec<IssueItem>,
    selected: usize,
    event_rx: mpsc::UnboundedReceiver<IssuesEvent>,
    event_tx: mpsc::UnboundedSender<IssuesEvent>,
    loaded: bool,
    status_msg: Option<String>,
}

impl IssuesPanel {
    pub fn new(client: Arc<GatewayClient>) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        IssuesPanel {
            client,
            items: Vec::new(),
            selected: 0,
            event_rx,
            event_tx,
            loaded: false,
            status_msg: None,
        }
    }

    fn load(&mut self) {
        let client = self.client.clone();
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            match client.request("issues.list", json!({})).await {
                Ok(resp) => {
                    let empty = vec![];
                    let raw = resp.get("issues").and_then(|v| v.as_array()).unwrap_or(&empty);
                    let items = raw.iter().filter_map(|v| {
                        Some(IssueItem {
                            id: v.get("id")?.as_str()?.to_string(),
                            agent_id: v.get("agent_id")?.as_str()?.to_string(),
                            level: v.get("level").and_then(|x| x.as_str()).unwrap_or("?").to_string(),
                            target: v.get("target").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                            msg: v.get("msg").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                            last_seen: v.get("last_seen").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                            count: v.get("count").and_then(|x| x.as_u64()).unwrap_or(1) as u32,
                            status: v.get("status").and_then(|x| x.as_str()).unwrap_or("open").to_string(),
                        })
                    }).collect();
                    let _ = tx.send(IssuesEvent::Loaded(items));
                }
                Err(e) => { let _ = tx.send(IssuesEvent::Error(e.to_string())); }
            }
        });
        self.status_msg = Some("Loading...".to_string());
    }

    fn action_ignore(&mut self) {
        let Some(item) = self.items.get(self.selected) else { return };
        if item.status != "open" {
            self.status_msg = Some("Already ignored".to_string());
            return;
        }
        let client = self.client.clone();
        let tx = self.event_tx.clone();
        let agent_id = item.agent_id.clone();
        let issue_id = item.id.clone();
        tokio::spawn(async move {
            match client.request("issues.ignore", json!({"agent_id": agent_id, "issue_id": issue_id})).await {
                Ok(_) => { let _ = tx.send(IssuesEvent::ActionDone(format!("Issue '{}' ignored", issue_id))); }
                Err(e) => { let _ = tx.send(IssuesEvent::Error(e.to_string())); }
            }
        });
        self.status_msg = Some("Ignoring...".to_string());
    }

    fn action_resolve(&mut self) {
        let Some(item) = self.items.get(self.selected) else { return };
        let client = self.client.clone();
        let tx = self.event_tx.clone();
        let agent_id = item.agent_id.clone();
        let issue_id = item.id.clone();
        tokio::spawn(async move {
            match client.request("issues.resolve", json!({"agent_id": agent_id, "issue_id": issue_id})).await {
                Ok(_) => { let _ = tx.send(IssuesEvent::ActionDone(format!("Issue '{}' resolved", issue_id))); }
                Err(e) => { let _ = tx.send(IssuesEvent::Error(e.to_string())); }
            }
        });
        self.status_msg = Some("Resolving...".to_string());
    }

    fn drain_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                IssuesEvent::Loaded(items) => {
                    self.items = items;
                    self.loaded = true;
                    if self.selected >= self.items.len() && !self.items.is_empty() {
                        self.selected = self.items.len() - 1;
                    }
                    self.status_msg = None;
                }
                IssuesEvent::ActionDone(msg) => {
                    self.status_msg = Some(msg);
                    self.load();
                }
                IssuesEvent::Error(e) => {
                    self.status_msg = Some(format!("Error: {}", e));
                    self.loaded = true;
                }
            }
        }
    }
}

impl Component for IssuesPanel {
    fn handle_event(&mut self, event: &KeyEvent) -> Action {
        if !self.loaded {
            self.load();
            self.loaded = true;
        }
        self.drain_events();
        match event.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.items.is_empty() {
                    self.selected = (self.selected + 1).min(self.items.len() - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Char('i') => self.action_ignore(),
            KeyCode::Char('d') | KeyCode::Char('x') => self.action_resolve(),
            KeyCode::Char('r') => self.load(),
            _ => {}
        }
        Action::None
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.drain_events();

        if !self.loaded {
            self.load();
            self.loaded = true;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1), Constraint::Length(1)])
            .split(area);

        let header = Row::new(vec!["ID", "Level", "Status", "Agent", "×", "Last seen", "Message"])
            .style(Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD))
            .bottom_margin(1);

        let rows: Vec<Row> = self.items.iter().enumerate().map(|(i, item)| {
            let is_sel = i == self.selected;
            let row_style = if is_sel {
                Style::default().fg(Theme::TEXT).bg(Theme::SURFACE0)
            } else {
                Style::default().fg(Theme::SUBTEXT0)
            };
            let level_style = match item.level.as_str() {
                "ERROR" => Style::default().fg(Theme::RED),
                "WARN" => Style::default().fg(Theme::YELLOW),
                _ => Style::default().fg(Theme::OVERLAY1),
            };
            let status_style = if item.status == "ignored" {
                Style::default().fg(Theme::OVERLAY0)
            } else {
                Style::default().fg(Theme::GREEN)
            };
            let last_ts = &item.last_seen[..19.min(item.last_seen.len())];
            let msg_line = if item.target.is_empty() {
                item.msg.clone()
            } else {
                format!("{} [{}]", item.msg, item.target)
            };
            Row::new(vec![
                Cell::from(item.id.clone()).style(row_style),
                Cell::from(item.level.clone()).style(level_style),
                Cell::from(item.status.clone()).style(status_style),
                Cell::from(item.agent_id.clone()).style(row_style),
                Cell::from(item.count.to_string()).style(Style::default().fg(Theme::OVERLAY1)),
                Cell::from(last_ts.to_string()).style(Style::default().fg(Theme::OVERLAY1)),
                Cell::from(msg_line).style(row_style),
            ])
        }).collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(10),
                Constraint::Length(7),
                Constraint::Length(8),
                Constraint::Length(12),
                Constraint::Length(4),
                Constraint::Length(19),
                Constraint::Min(0),
            ],
        )
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Theme::SURFACE1))
                .title(format!(" 🔥 Issues ({}) ", self.items.len()))
                .title_style(Style::default().fg(Theme::RED)),
        );

        frame.render_widget(table, chunks[0]);

        let status = if let Some(msg) = &self.status_msg {
            Paragraph::new(format!(" {}", msg))
                .style(Style::default().fg(Theme::GREEN).bg(Theme::MANTLE))
        } else {
            Paragraph::new("").style(Style::default().bg(Theme::MANTLE))
        };
        frame.render_widget(status, chunks[1]);

        let help = Paragraph::new(" i Ignore  d/x Resolve  r Reload")
            .style(Style::default().fg(Theme::OVERLAY0).bg(Theme::MANTLE));
        frame.render_widget(help, chunks[2]);
    }
}
