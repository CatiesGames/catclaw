//! Contacts panel — list contacts + drafts; toggle ai_paused/approval; approve/discard drafts.

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

#[derive(Debug, Clone)]
struct ContactItem {
    id: String,
    #[allow(dead_code)]
    agent_id: String,
    display_name: String,
    role: String,
    tags: Vec<String>,
    forward_channel: Option<String>,
    approval_required: bool,
    ai_paused: bool,
}

#[derive(Debug, Clone)]
struct DraftItem {
    id: i64,
    contact_id: String,
    status: String,
    payload_preview: String,
    #[allow(dead_code)]
    created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubTab {
    Contacts,
    Drafts,
}

enum Event {
    ContactsLoaded(Vec<ContactItem>),
    DraftsLoaded(Vec<DraftItem>),
    ActionDone(String),
}

pub struct ContactsPanel {
    client: Arc<GatewayClient>,
    contacts: Vec<ContactItem>,
    drafts: Vec<DraftItem>,
    selected: usize,
    sub: SubTab,
    event_rx: mpsc::UnboundedReceiver<Event>,
    event_tx: mpsc::UnboundedSender<Event>,
    contacts_loaded: bool,
    drafts_loaded: bool,
    status_msg: Option<String>,
}

impl ContactsPanel {
    pub fn new(client: Arc<GatewayClient>) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        ContactsPanel {
            client,
            contacts: Vec::new(),
            drafts: Vec::new(),
            selected: 0,
            sub: SubTab::Contacts,
            event_rx,
            event_tx,
            contacts_loaded: false,
            drafts_loaded: false,
            status_msg: None,
        }
    }

    fn refresh_contacts(&mut self) {
        let client = self.client.clone();
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            match client.request("contact.list", json!({})).await {
                Ok(v) => {
                    let _ = tx.send(Event::ContactsLoaded(parse_contacts(&v)));
                }
                Err(e) => error!(error = %e, "contacts: load failed"),
            }
        });
    }

    fn refresh_drafts(&mut self) {
        let client = self.client.clone();
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            match client
                .request("contact.draft.list", json!({"limit": 100}))
                .await
            {
                Ok(v) => {
                    let _ = tx.send(Event::DraftsLoaded(parse_drafts(&v)));
                }
                Err(e) => error!(error = %e, "contacts drafts: load failed"),
            }
        });
    }

    fn refresh(&mut self) {
        match self.sub {
            SubTab::Contacts => self.refresh_contacts(),
            SubTab::Drafts => self.refresh_drafts(),
        }
    }

    fn current_contact(&self) -> Option<&ContactItem> {
        self.contacts.get(self.selected)
    }

    fn toggle_pause(&mut self) {
        if let Some(c) = self.current_contact().cloned() {
            let id = c.id.clone();
            let method = if c.ai_paused {
                "contact.ai_resume"
            } else {
                "contact.ai_pause"
            };
            let client = self.client.clone();
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                match client.request(method, json!({"id": id})).await {
                    Ok(_) => {
                        let _ = tx.send(Event::ActionDone(format!("toggled ai_paused for {}", id)));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::ActionDone(format!("error: {}", e)));
                    }
                }
            });
        }
    }

    fn toggle_approval(&mut self) {
        if let Some(c) = self.current_contact().cloned() {
            let id = c.id.clone();
            let new_val = !c.approval_required;
            let client = self.client.clone();
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                let r = client
                    .request(
                        "contact.update",
                        json!({"id": id, "approval_required": new_val}),
                    )
                    .await;
                match r {
                    Ok(_) => {
                        let _ = tx.send(Event::ActionDone(format!(
                            "approval_required → {} for {}",
                            new_val, id
                        )));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::ActionDone(format!("error: {}", e)));
                    }
                }
            });
        }
    }

    fn approve_draft(&mut self) {
        if let Some(d) = self.drafts.get(self.selected).cloned() {
            let id = d.id;
            let client = self.client.clone();
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                let r = client
                    .request("contact.draft.approve", json!({"id": id}))
                    .await;
                match r {
                    Ok(_) => {
                        let _ = tx.send(Event::ActionDone(format!("draft {} approved", id)));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::ActionDone(format!("error: {}", e)));
                    }
                }
            });
        }
    }

    fn discard_draft(&mut self) {
        if let Some(d) = self.drafts.get(self.selected).cloned() {
            let id = d.id;
            let client = self.client.clone();
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                let r = client
                    .request("contact.draft.discard", json!({"id": id}))
                    .await;
                match r {
                    Ok(_) => {
                        let _ = tx.send(Event::ActionDone(format!("draft {} discarded", id)));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::ActionDone(format!("error: {}", e)));
                    }
                }
            });
        }
    }
}

impl Component for ContactsPanel {
    fn handle_event(&mut self, event: &KeyEvent) -> Action {
        while let Ok(ev) = self.event_rx.try_recv() {
            match ev {
                Event::ContactsLoaded(c) => {
                    self.contacts = c;
                    self.contacts_loaded = true;
                    if self.selected >= self.contacts.len() && !self.contacts.is_empty() {
                        self.selected = self.contacts.len() - 1;
                    }
                }
                Event::DraftsLoaded(d) => {
                    self.drafts = d;
                    self.drafts_loaded = true;
                    if self.selected >= self.drafts.len() && !self.drafts.is_empty() {
                        self.selected = self.drafts.len() - 1;
                    }
                }
                Event::ActionDone(msg) => {
                    self.status_msg = Some(msg);
                    self.refresh();
                }
            }
        }
        self.status_msg = None;

        match event.code {
            KeyCode::Char('r') | KeyCode::F(5) => self.refresh(),
            KeyCode::Tab | KeyCode::Char('\t') => {
                self.sub = match self.sub {
                    SubTab::Contacts => SubTab::Drafts,
                    SubTab::Drafts => SubTab::Contacts,
                };
                self.selected = 0;
                self.refresh();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = match self.sub {
                    SubTab::Contacts => self.contacts.len(),
                    SubTab::Drafts => self.drafts.len(),
                };
                if self.selected + 1 < len {
                    self.selected += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') if self.selected > 0 => {
                self.selected -= 1;
            }
            KeyCode::Char('p') | KeyCode::Char('P') if self.sub == SubTab::Contacts => {
                self.toggle_pause();
                self.status_msg = Some("toggling ai_paused…".to_string());
            }
            KeyCode::Char('A') if self.sub == SubTab::Contacts => {
                self.toggle_approval();
                self.status_msg = Some("toggling approval_required…".to_string());
            }
            KeyCode::Char('a') if self.sub == SubTab::Drafts => {
                self.approve_draft();
                self.status_msg = Some("approving draft…".to_string());
            }
            KeyCode::Char('d') | KeyCode::Char('D') if self.sub == SubTab::Drafts => {
                self.discard_draft();
                self.status_msg = Some("discarding draft…".to_string());
            }
            _ => {}
        }

        if self.sub == SubTab::Contacts && !self.contacts_loaded {
            self.refresh_contacts();
        }
        if self.sub == SubTab::Drafts && !self.drafts_loaded {
            self.refresh_drafts();
        }

        Action::None
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Drain any pending events.
        while let Ok(ev) = self.event_rx.try_recv() {
            match ev {
                Event::ContactsLoaded(c) => {
                    self.contacts = c;
                    self.contacts_loaded = true;
                }
                Event::DraftsLoaded(d) => {
                    self.drafts = d;
                    self.drafts_loaded = true;
                }
                Event::ActionDone(msg) => {
                    self.status_msg = Some(msg);
                    self.refresh();
                }
            }
        }
        if self.sub == SubTab::Contacts && !self.contacts_loaded {
            self.refresh_contacts();
        }
        if self.sub == SubTab::Drafts && !self.drafts_loaded {
            self.refresh_drafts();
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(6),
                Constraint::Length(1),
            ])
            .split(area);

        // Tab bar
        let (label_c, label_d) = match self.sub {
            SubTab::Contacts => (
                Span::styled(
                    " [Contacts] ",
                    Style::default()
                        .fg(Theme::MAUVE)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Drafts ", Style::default().fg(Theme::OVERLAY1)),
            ),
            SubTab::Drafts => (
                Span::styled(" Contacts ", Style::default().fg(Theme::OVERLAY1)),
                Span::styled(
                    " [Drafts] ",
                    Style::default()
                        .fg(Theme::MAUVE)
                        .add_modifier(Modifier::BOLD),
                ),
            ),
        };
        let bar = Paragraph::new(Line::from(vec![label_c, Span::raw("  "), label_d]))
            .block(Block::default().borders(Borders::ALL).title(" Contacts "));
        frame.render_widget(bar, chunks[0]);

        match self.sub {
            SubTab::Contacts => self.render_contacts_table(frame, chunks[1]),
            SubTab::Drafts => self.render_drafts_table(frame, chunks[1]),
        }

        // Hints / status
        let (txt, st) = if let Some(ref m) = self.status_msg {
            (m.clone(), Style::default().fg(Theme::GREEN).bg(Theme::MANTLE))
        } else {
            let h = match self.sub {
                SubTab::Contacts =>
                    " Tab Drafts  P Pause/Resume  A Toggle approval  r Refresh".to_string(),
                SubTab::Drafts =>
                    " Tab Contacts  a Approve  D Discard  r Refresh".to_string(),
            };
            (h, Style::default().fg(Theme::MAUVE).bg(Theme::MANTLE))
        };
        frame.render_widget(Paragraph::new(txt).style(st), chunks[2]);
    }
}

impl ContactsPanel {
    fn render_contacts_table(&self, frame: &mut Frame, area: Rect) {
        let header = Row::new(vec!["Name", "Role", "Tags", "Forward", "Approve", "Paused"])
            .style(Style::default().add_modifier(Modifier::BOLD));
        let rows: Vec<Row> = self
            .contacts
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let style = if i == self.selected {
                    Style::default().bg(Theme::MAUVE).fg(Theme::BASE)
                } else {
                    Style::default()
                };
                Row::new(vec![
                    c.display_name.clone(),
                    c.role.clone(),
                    c.tags.join(","),
                    c.forward_channel.clone().unwrap_or_else(|| "-".into()),
                    if c.approval_required { "yes".into() } else { "no".into() },
                    if c.ai_paused { "yes".into() } else { "no".into() },
                ])
                .style(style)
            })
            .collect();
        let widths = [
            Constraint::Min(14),
            Constraint::Length(8),
            Constraint::Min(14),
            Constraint::Min(20),
            Constraint::Length(8),
            Constraint::Length(8),
        ];
        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(table, area);
    }

    fn render_drafts_table(&self, frame: &mut Frame, area: Rect) {
        let header = Row::new(vec!["ID", "Contact", "Status", "Payload"])
            .style(Style::default().add_modifier(Modifier::BOLD));
        let rows: Vec<Row> = self
            .drafts
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let style = if i == self.selected {
                    Style::default().bg(Theme::MAUVE).fg(Theme::BASE)
                } else {
                    Style::default()
                };
                let preview: String = d.payload_preview.chars().take(60).collect();
                Row::new(vec![
                    d.id.to_string(),
                    short_id(&d.contact_id),
                    d.status.clone(),
                    preview,
                ])
                .style(style)
            })
            .collect();
        let widths = [
            Constraint::Length(5),
            Constraint::Length(10),
            Constraint::Length(20),
            Constraint::Min(20),
        ];
        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(table, area);
    }
}

fn short_id(s: &str) -> String {
    if s.len() <= 8 {
        s.to_string()
    } else {
        s[..8].to_string()
    }
}

fn parse_contacts(val: &serde_json::Value) -> Vec<ContactItem> {
    val.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    Some(ContactItem {
                        id: v.get("id")?.as_str()?.to_string(),
                        agent_id: v.get("agent_id")?.as_str()?.to_string(),
                        display_name: v.get("display_name")?.as_str()?.to_string(),
                        role: v.get("role")?.as_str()?.to_string(),
                        tags: v
                            .get("tags")
                            .and_then(|x| x.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|t| t.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        forward_channel: v.get("forward_channel").and_then(|x| x.as_str()).map(String::from),
                        approval_required: v.get("approval_required").and_then(|x| x.as_bool()).unwrap_or(true),
                        ai_paused: v.get("ai_paused").and_then(|x| x.as_bool()).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_drafts(val: &serde_json::Value) -> Vec<DraftItem> {
    val.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    let payload = v.get("payload").cloned().unwrap_or_default();
                    let preview = payload.get("text").and_then(|x| x.as_str())
                        .map(String::from)
                        .unwrap_or_else(|| serde_json::to_string(&payload).unwrap_or_default());
                    Some(DraftItem {
                        id: v.get("id")?.as_i64()?,
                        contact_id: v.get("contact_id")?.as_str()?.to_string(),
                        status: v.get("status")?.as_str()?.to_string(),
                        payload_preview: preview,
                        created_at: v.get("created_at").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
