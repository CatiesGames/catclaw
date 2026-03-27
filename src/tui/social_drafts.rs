use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;
use serde_json::json;
use tokio::sync::mpsc;
use tracing::error;

use super::theme::Theme;
use super::{Action, Component};
use crate::ws_client::GatewayClient;

/// Social draft row as returned by WS social.draft.list
#[derive(Debug, Clone)]
struct DraftItem {
    id: i64,
    platform: String,
    draft_type: String,
    content: String,
    media_url: Option<String>,
    status: String,
    created_at: String,
}

#[derive(Debug)]
enum StatusFilter {
    All,
    Draft,
    AwaitingApproval,
    Sent,
    Ignored,
    Failed,
}

impl StatusFilter {
    fn as_str(&self) -> Option<&str> {
        match self {
            StatusFilter::All => None,
            StatusFilter::Draft => Some("draft"),
            StatusFilter::AwaitingApproval => Some("awaiting_approval"),
            StatusFilter::Sent => Some("sent"),
            StatusFilter::Ignored => Some("ignored"),
            StatusFilter::Failed => Some("failed"),
        }
    }

    fn label(&self) -> &str {
        match self {
            StatusFilter::All => "All",
            StatusFilter::Draft => "Draft",
            StatusFilter::AwaitingApproval => "Pending",
            StatusFilter::Sent => "Sent",
            StatusFilter::Ignored => "Ignored",
            StatusFilter::Failed => "Failed",
        }
    }

    fn all() -> &'static [StatusFilter] {
        &[
            StatusFilter::All,
            StatusFilter::Draft,
            StatusFilter::AwaitingApproval,
            StatusFilter::Sent,
            StatusFilter::Ignored,
            StatusFilter::Failed,
        ]
    }
}

enum DraftEvent {
    Loaded(Vec<DraftItem>),
    ActionDone(String),
}

pub struct SocialDraftsPanel {
    client: Arc<GatewayClient>,
    items: Vec<DraftItem>,
    selected: usize,
    filter_idx: usize,
    event_rx: mpsc::UnboundedReceiver<DraftEvent>,
    event_tx: mpsc::UnboundedSender<DraftEvent>,
    loaded: bool,
    status_msg: Option<String>,
    detail_view: bool,
}

impl SocialDraftsPanel {
    pub fn new(client: Arc<GatewayClient>) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        SocialDraftsPanel {
            client,
            items: Vec::new(),
            selected: 0,
            filter_idx: 0,
            event_rx,
            event_tx,
            loaded: false,
            status_msg: None,
            detail_view: false,
        }
    }

    fn refresh(&mut self) {
        let client = self.client.clone();
        let tx = self.event_tx.clone();
        let status = StatusFilter::all()[self.filter_idx].as_str().map(str::to_string);
        tokio::spawn(async move {
            let mut params = json!({ "limit": 50 });
            if let Some(s) = status {
                params["status"] = json!(s);
            }
            match client.request("social.draft.list", params).await {
                Ok(val) => {
                    let items = parse_items(&val);
                    let _ = tx.send(DraftEvent::Loaded(items));
                }
                Err(e) => {
                    error!(error = %e, "social drafts: failed to load items");
                }
            }
        });
    }

    fn approve(&mut self) {
        if let Some(item) = self.items.get(self.selected) {
            let id = item.id;
            let client = self.client.clone();
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                match client.request("social.draft.approve", json!({ "id": id })).await {
                    Ok(_) => { let _ = tx.send(DraftEvent::ActionDone(format!("Draft {} approved and sent", id))); }
                    Err(e) => { let _ = tx.send(DraftEvent::ActionDone(format!("Error: {}", e))); }
                }
            });
        }
    }

    fn discard(&mut self) {
        if let Some(item) = self.items.get(self.selected) {
            let id = item.id;
            let client = self.client.clone();
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                match client.request("social.draft.discard", json!({ "id": id })).await {
                    Ok(_) => { let _ = tx.send(DraftEvent::ActionDone(format!("Draft {} discarded", id))); }
                    Err(e) => { let _ = tx.send(DraftEvent::ActionDone(format!("Error: {}", e))); }
                }
            });
        }
    }
}

impl Component for SocialDraftsPanel {
    fn captures_input(&self) -> bool {
        false
    }

    fn handle_event(&mut self, event: &KeyEvent) -> Action {
        // Drain async events first.
        while let Ok(ev) = self.event_rx.try_recv() {
            match ev {
                DraftEvent::Loaded(items) => {
                    self.items = items;
                    self.loaded = true;
                    if self.selected >= self.items.len() && !self.items.is_empty() {
                        self.selected = self.items.len() - 1;
                    }
                }
                DraftEvent::ActionDone(msg) => {
                    self.status_msg = Some(msg);
                    self.refresh();
                }
            }
        }

        // Clear status message on any key press
        self.status_msg = None;

        if self.detail_view {
            // Detail view: limited key handling
            match event.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                    self.detail_view = false;
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    self.approve();
                    self.status_msg = Some("Approving draft…".to_string());
                }
                KeyCode::Char('d') | KeyCode::Char('D') => {
                    self.discard();
                    self.status_msg = Some("Discarding draft…".to_string());
                    self.detail_view = false;
                }
                _ => {}
            }
        } else {
            // List view
            match event.code {
                KeyCode::Char('r') | KeyCode::F(5) => {
                    self.refresh();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.selected + 1 < self.items.len() {
                        self.selected += 1;
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.selected > 0 {
                        self.selected -= 1;
                    }
                }
                KeyCode::Enter => {
                    self.detail_view = true;
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    self.approve();
                    self.status_msg = Some("Approving draft…".to_string());
                }
                KeyCode::Char('d') | KeyCode::Char('D') => {
                    self.discard();
                    self.status_msg = Some("Discarding draft…".to_string());
                }
                KeyCode::Tab => {
                    self.filter_idx = (self.filter_idx + 1) % StatusFilter::all().len();
                    self.selected = 0;
                    self.refresh();
                }
                KeyCode::BackTab => {
                    self.filter_idx = (self.filter_idx + StatusFilter::all().len() - 1) % StatusFilter::all().len();
                    self.selected = 0;
                    self.refresh();
                }
                _ => {}
            }
        }

        if !self.loaded {
            self.refresh();
        }

        Action::None
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        while let Ok(ev) = self.event_rx.try_recv() {
            match ev {
                DraftEvent::Loaded(items) => {
                    self.items = items;
                    self.loaded = true;
                }
                DraftEvent::ActionDone(msg) => {
                    self.status_msg = Some(msg);
                    self.refresh();
                }
            }
        }
        if !self.loaded {
            self.refresh();
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(6),
                Constraint::Length(1),
            ])
            .split(area);

        // ── Filter bar ────────────────────────────────────────────────────────
        let filter_spans: Vec<Span> = StatusFilter::all()
            .iter()
            .enumerate()
            .flat_map(|(i, f)| {
                let label = if i == self.filter_idx {
                    Span::styled(
                        format!(" [{}] ", f.label()),
                        Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::styled(format!(" {} ", f.label()), Style::default().fg(Theme::OVERLAY1))
                };
                vec![label, Span::raw(" ")]
            })
            .collect();
        let filter_bar = Paragraph::new(Line::from(filter_spans))
            .block(Block::default().borders(Borders::ALL).title(" Social Drafts "));
        frame.render_widget(filter_bar, chunks[0]);

        // ── Draft list / detail ───────────────────────────────────────────────
        if self.detail_view {
            if let Some(item) = self.items.get(self.selected) {
                let mut detail_text = vec![
                    Line::from(vec![
                        Span::styled("Platform: ", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw(format!("{} ({})", item.platform, item.draft_type)),
                    ]),
                    Line::from(vec![
                        Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw(item.status.clone()),
                    ]),
                    Line::from(vec![
                        Span::styled("Content: ", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw(item.content.clone()),
                    ]),
                ];
                if let Some(ref url) = item.media_url {
                    detail_text.push(Line::from(vec![
                        Span::styled("Media: ", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw(url.to_string()),
                    ]));
                }
                detail_text.push(Line::from(""));
                detail_text.push(Line::from(vec![
                    Span::styled("Created: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(item.created_at.clone()),
                ]));
                let detail = Paragraph::new(detail_text)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!(" Draft #{} ", item.id)),
                    )
                    .wrap(Wrap { trim: false });
                frame.render_widget(detail, chunks[1]);
            }
        } else {
            let header = Row::new(vec!["ID", "Platform", "Type", "Content", "Status"])
                .style(Style::default().add_modifier(Modifier::BOLD));
            let rows: Vec<Row> = self
                .items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let mut content_preview: String = item.content.chars().take(40).collect();
                    if item.media_url.is_some() {
                        content_preview = format!("[img] {}", content_preview);
                    }
                    let style = if i == self.selected {
                        Style::default().bg(Theme::MAUVE).fg(Theme::BASE)
                    } else {
                        Style::default()
                    };
                    Row::new(vec![
                        item.id.to_string(),
                        item.platform.clone(),
                        item.draft_type.clone(),
                        content_preview,
                        item.status.clone(),
                    ])
                    .style(style)
                })
                .collect();

            let widths = [
                Constraint::Length(5),   // ID
                Constraint::Length(10),  // Platform
                Constraint::Length(6),   // Type
                Constraint::Min(20),    // Content
                Constraint::Length(20),  // Status
            ];
            let table = Table::new(rows, widths)
                .header(header)
                .block(Block::default().borders(Borders::ALL));
            frame.render_widget(table, chunks[1]);
        }

        // ── Hints bar ─────────────────────────────────────────────────────────
        let (hint_text, hint_style) = if let Some(ref msg) = self.status_msg {
            (msg.clone(), Style::default().fg(Theme::GREEN).bg(Theme::MANTLE))
        } else if self.detail_view {
            (" Esc Back  A Approve  D Discard".to_string(), Style::default().fg(Theme::TEXT).bg(Theme::MANTLE))
        } else {
            (" Enter Detail  A Approve  D Discard  Tab Filter  r Refresh".to_string(), Style::default().fg(Theme::TEXT).bg(Theme::MANTLE))
        };
        let hints = Paragraph::new(hint_text).style(hint_style);
        frame.render_widget(hints, chunks[2]);
    }
}

fn parse_items(val: &serde_json::Value) -> Vec<DraftItem> {
    val.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    Some(DraftItem {
                        id: v.get("id")?.as_i64()?,
                        platform: v.get("platform")?.as_str()?.to_string(),
                        draft_type: v.get("draft_type")?.as_str()?.to_string(),
                        content: v.get("content")?.as_str()?.to_string(),
                        media_url: v.get("media_url").and_then(|x| x.as_str()).map(str::to_string),
                        status: v.get("status")?.as_str()?.to_string(),
                        created_at: v.get("created_at").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
