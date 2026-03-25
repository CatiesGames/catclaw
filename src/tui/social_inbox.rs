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

/// Social inbox row as returned by WS social.inbox.list
#[derive(Debug, Clone)]
struct InboxItem {
    id: i64,
    platform: String,
    event_type: String,
    author_name: String,
    text: String,
    status: String,
    draft: Option<String>,
    created_at: String,
}

#[derive(Debug)]
enum StatusFilter {
    All,
    Pending,
    Forwarded,
    DraftReady,
    Sent,
    Ignored,
}

impl StatusFilter {
    fn as_str(&self) -> Option<&str> {
        match self {
            StatusFilter::All => None,
            StatusFilter::Pending => Some("pending"),
            StatusFilter::Forwarded => Some("forwarded"),
            StatusFilter::DraftReady => Some("draft_ready"),
            StatusFilter::Sent => Some("sent"),
            StatusFilter::Ignored => Some("ignored"),
        }
    }

    fn label(&self) -> &str {
        match self {
            StatusFilter::All => "All",
            StatusFilter::Pending => "Pending",
            StatusFilter::Forwarded => "Forwarded",
            StatusFilter::DraftReady => "Draft",
            StatusFilter::Sent => "Sent",
            StatusFilter::Ignored => "Ignored",
        }
    }

    fn all() -> &'static [StatusFilter] {
        &[
            StatusFilter::All,
            StatusFilter::Pending,
            StatusFilter::Forwarded,
            StatusFilter::DraftReady,
            StatusFilter::Sent,
            StatusFilter::Ignored,
        ]
    }
}

enum InboxEvent {
    Loaded(Vec<InboxItem>),
    ActionDone(String),
}

pub struct SocialInboxPanel {
    client: Arc<GatewayClient>,
    items: Vec<InboxItem>,
    selected: usize,
    filter_idx: usize,
    event_rx: mpsc::UnboundedReceiver<InboxEvent>,
    event_tx: mpsc::UnboundedSender<InboxEvent>,
    loaded: bool,
    status_msg: Option<String>,
    detail_view: bool,
}

impl SocialInboxPanel {
    pub fn new(client: Arc<GatewayClient>) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        SocialInboxPanel {
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

    #[allow(dead_code)]
    fn current_filter(&self) -> &StatusFilter {
        &StatusFilter::all()[self.filter_idx]
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
            match client.request("social.inbox.list", params).await {
                Ok(val) => {
                    let items = parse_items(&val);
                    let _ = tx.send(InboxEvent::Loaded(items));
                }
                Err(e) => {
                    error!(error = %e, "social inbox: failed to load items");
                }
            }
        });
    }

    fn approve_draft(&mut self) {
        if let Some(item) = self.items.get(self.selected) {
            let id = item.id;
            let client = self.client.clone();
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                match client.request("social.inbox.approve", json!({ "id": id })).await {
                    Ok(_) => { let _ = tx.send(InboxEvent::ActionDone(format!("Draft {} approved and sent", id))); }
                    Err(e) => { let _ = tx.send(InboxEvent::ActionDone(format!("Error: {}", e))); }
                }
            });
        }
    }

    fn discard_draft(&mut self) {
        if let Some(item) = self.items.get(self.selected) {
            let id = item.id;
            let client = self.client.clone();
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                match client.request("social.inbox.discard", json!({ "id": id })).await {
                    Ok(_) => { let _ = tx.send(InboxEvent::ActionDone(format!("Draft {} discarded", id))); }
                    Err(e) => { let _ = tx.send(InboxEvent::ActionDone(format!("Error: {}", e))); }
                }
            });
        }
    }

    fn reprocess(&mut self) {
        if let Some(item) = self.items.get(self.selected) {
            let id = item.id;
            let client = self.client.clone();
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                match client.request("social.inbox.reprocess", json!({ "id": id })).await {
                    Ok(_) => { let _ = tx.send(InboxEvent::ActionDone(format!("Item {} reset to pending", id))); }
                    Err(e) => { let _ = tx.send(InboxEvent::ActionDone(format!("Error: {}", e))); }
                }
            });
        }
    }

    fn poll_now(&mut self) {
        let client = self.client.clone();
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            match client.request("social.poll", json!({})).await {
                Ok(_) => { let _ = tx.send(InboxEvent::ActionDone("Poll triggered".to_string())); }
                Err(e) => { let _ = tx.send(InboxEvent::ActionDone(format!("Poll error: {}", e))); }
            }
        });
    }
}

impl Component for SocialInboxPanel {
    fn captures_input(&self) -> bool {
        false
    }

    fn handle_event(&mut self, event: &KeyEvent) -> Action {
        // Drain async events first.
        while let Ok(ev) = self.event_rx.try_recv() {
            match ev {
                InboxEvent::Loaded(items) => {
                    self.items = items;
                    self.loaded = true;
                    if self.selected >= self.items.len() && !self.items.is_empty() {
                        self.selected = self.items.len() - 1;
                    }
                }
                InboxEvent::ActionDone(msg) => {
                    self.status_msg = Some(msg);
                    self.refresh();
                }
            }
        }

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
                self.detail_view = !self.detail_view;
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.approve_draft();
                self.status_msg = Some("Approving draft…".to_string());
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                self.discard_draft();
                self.status_msg = Some("Discarding draft…".to_string());
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                self.poll_now();
                self.status_msg = Some("Polling…".to_string());
            }
            KeyCode::Char('R') => {
                self.reprocess();
                self.status_msg = Some("Reprocessing…".to_string());
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

        // Load on first render.
        if !self.loaded {
            self.refresh();
        }

        Action::None
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Drain async events on render too.
        while let Ok(ev) = self.event_rx.try_recv() {
            match ev {
                InboxEvent::Loaded(items) => {
                    self.items = items;
                    self.loaded = true;
                }
                InboxEvent::ActionDone(msg) => {
                    self.status_msg = Some(msg);
                    self.refresh();
                }
            }
        }
        if !self.loaded {
            self.refresh();
        }

        // Split: header / list / detail / footer.
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // filter bar
                Constraint::Min(6),     // list
                Constraint::Length(1),  // hints
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
            .block(Block::default().borders(Borders::ALL).title(" Social Inbox "));
        frame.render_widget(filter_bar, chunks[0]);

        // ── Item list ─────────────────────────────────────────────────────────
        if self.detail_view {
            if let Some(item) = self.items.get(self.selected) {
                let detail_text = vec![
                    Line::from(vec![
                        Span::styled("Platform: ", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw(format!("{} ({})", item.platform, item.event_type)),
                    ]),
                    Line::from(vec![
                        Span::styled("From: ", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw(item.author_name.clone()),
                    ]),
                    Line::from(vec![
                        Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw(item.status.clone()),
                    ]),
                    Line::from(vec![
                        Span::styled("Text: ", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw(item.text.clone()),
                    ]),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("Draft: ", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw(item.draft.as_deref().unwrap_or("(none)")),
                    ]),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("Created: ", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw(item.created_at.clone()),
                    ]),
                ];
                let detail = Paragraph::new(detail_text)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!(" Item #{} ", item.id)),
                    )
                    .wrap(Wrap { trim: false });
                frame.render_widget(detail, chunks[1]);
            }
        } else {
            let header = Row::new(vec!["ID", "Platform", "Type", "From", "Text", "Status"])
                .style(Style::default().add_modifier(Modifier::BOLD));
            let rows: Vec<Row> = self
                .items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let text_preview: String = item.text.chars().take(32).collect();
                    let style = if i == self.selected {
                        Style::default().bg(Theme::MAUVE).fg(Theme::BASE)
                    } else {
                        Style::default()
                    };
                    Row::new(vec![
                        item.id.to_string(),
                        item.platform.clone(),
                        item.event_type.clone(),
                        item.author_name.clone(),
                        text_preview,
                        item.status.clone(),
                    ])
                    .style(style)
                })
                .collect();

            let widths = [
                Constraint::Length(5),
                Constraint::Length(12),
                Constraint::Length(10),
                Constraint::Length(16),
                Constraint::Min(20),
                Constraint::Length(12),
            ];
            let table = Table::new(rows, widths)
                .header(header)
                .block(Block::default().borders(Borders::ALL));
            frame.render_widget(table, chunks[1]);
        }

        // ── Hints bar ─────────────────────────────────────────────────────────
        let hint_text = if let Some(ref msg) = self.status_msg {
            msg.clone()
        } else {
            "[Enter] Detail  [A] Approve  [D] Discard  [R] Reprocess  [P] Poll  [Tab] Filter  [r] Refresh".to_string()
        };
        let hints = Paragraph::new(hint_text)
            .style(Style::default().fg(Theme::OVERLAY1));
        frame.render_widget(hints, chunks[2]);
    }
}

fn parse_items(val: &serde_json::Value) -> Vec<InboxItem> {
    val.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    Some(InboxItem {
                        id: v.get("id")?.as_i64()?,
                        platform: v.get("platform")?.as_str()?.to_string(),
                        event_type: v.get("event_type")?.as_str()?.to_string(),
                        author_name: v.get("author_name").and_then(|x| x.as_str()).unwrap_or("-").to_string(),
                        text: v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        status: v.get("status")?.as_str()?.to_string(),
                        draft: v.get("draft").and_then(|x| x.as_str()).map(str::to_string),
                        created_at: v.get("created_at").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
