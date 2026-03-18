use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;
use serde_json::json;
use tokio::sync::mpsc;
use tracing::error;

use super::theme::Theme;
use super::{local_timezone_label, utc_to_local_display, Action, Component};
use crate::ws_client::GatewayClient;

/// Task info from WebSocket
#[derive(Debug, Clone)]
struct TaskInfo {
    id: i64,
    name: String,
    agent_id: String,
    cron_expr: Option<String>,
    interval_mins: Option<i64>,
    next_run_at: String,
    enabled: bool,
}

enum TaskEvent {
    Loaded(Vec<TaskInfo>),
}

pub struct TasksPanel {
    client: Arc<GatewayClient>,
    tasks: Vec<TaskInfo>,
    selected: usize,
    event_rx: mpsc::UnboundedReceiver<TaskEvent>,
    event_tx: mpsc::UnboundedSender<TaskEvent>,
    loaded: bool,
}

impl TasksPanel {
    pub fn new(client: Arc<GatewayClient>) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        TasksPanel {
            client,
            tasks: Vec::new(),
            selected: 0,
            event_rx,
            event_tx,
            loaded: false,
        }
    }

    fn refresh(&mut self) {
        let client = self.client.clone();
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            match client.request("tasks.list", json!({})).await {
                Ok(val) => {
                    let tasks = parse_tasks(&val);
                    let _ = tx.send(TaskEvent::Loaded(tasks));
                }
                Err(e) => {
                    error!(error = %e, "failed to fetch tasks");
                }
            }
        });
    }

    fn poll_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                TaskEvent::Loaded(tasks) => {
                    self.tasks = tasks;
                    if self.selected >= self.tasks.len() && !self.tasks.is_empty() {
                        self.selected = self.tasks.len() - 1;
                    }
                }
            }
        }
    }

    fn toggle_selected(&mut self) {
        if let Some(task) = self.tasks.get(self.selected) {
            let id = task.id;
            let method = if task.enabled {
                "tasks.disable"
            } else {
                "tasks.enable"
            };
            let client = self.client.clone();
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                let _ = client.request(method, json!({ "id": id })).await;
                // Refresh after toggle
                if let Ok(val) = client.request("tasks.list", json!({})).await {
                    let _ = tx.send(TaskEvent::Loaded(parse_tasks(&val)));
                }
            });
        }
    }

    fn delete_selected(&mut self) {
        if let Some(task) = self.tasks.get(self.selected) {
            let id = task.id;
            let client = self.client.clone();
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                let _ = client.request("tasks.delete", json!({ "id": id })).await;
                if let Ok(val) = client.request("tasks.list", json!({})).await {
                    let _ = tx.send(TaskEvent::Loaded(parse_tasks(&val)));
                }
            });
        }
    }

    fn format_schedule(task: &TaskInfo) -> String {
        if let Some(ref cron) = task.cron_expr {
            format!("cron: {}", cron)
        } else if let Some(mins) = task.interval_mins {
            if mins >= 1440 {
                format!("every {}d", mins / 1440)
            } else if mins >= 60 {
                format!("every {}h", mins / 60)
            } else {
                format!("every {}m", mins)
            }
        } else {
            "one-shot".to_string()
        }
    }

    fn format_next_run(task: &TaskInfo) -> String {
        utc_to_local_display(&task.next_run_at)
    }
}

fn parse_tasks(val: &serde_json::Value) -> Vec<TaskInfo> {
    let arr = match val.as_array() {
        Some(a) => a,
        None => return vec![],
    };
    arr.iter()
        .filter_map(|v| {
            Some(TaskInfo {
                id: v.get("id")?.as_i64()?,
                name: v.get("name")?.as_str()?.to_string(),
                agent_id: v.get("agent_id")?.as_str()?.to_string(),
                cron_expr: v.get("cron_expr").and_then(|v| v.as_str()).map(String::from),
                interval_mins: v.get("interval_mins").and_then(|v| v.as_i64()),
                next_run_at: v.get("next_run_at")?.as_str()?.to_string(),
                enabled: v.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
            })
        })
        .collect()
}

impl Component for TasksPanel {
    fn handle_event(&mut self, event: &KeyEvent) -> Action {
        match event.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.tasks.is_empty() {
                    self.selected = (self.selected + 1).min(self.tasks.len() - 1);
                }
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                Action::None
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                self.toggle_selected();
                Action::None
            }
            KeyCode::Char('d') => {
                self.delete_selected();
                Action::None
            }
            KeyCode::Char('r') => {
                self.refresh();
                Action::None
            }
            _ => Action::None,
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        if !self.loaded {
            self.loaded = true;
            self.refresh();
        }
        self.poll_events();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);

        if self.tasks.is_empty() {
            let msg = Paragraph::new("\n  No scheduled tasks.\n\n  Add tasks with: catclaw task add <name> --prompt \"...\"")
                .style(Style::default().fg(Theme::OVERLAY0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Theme::SURFACE1))
                        .title(" Tasks ")
                        .title_style(Style::default().fg(Theme::MAUVE)),
                );
            frame.render_widget(msg, chunks[0]);
        } else {
            let header = Row::new(vec!["", "ID", "Name", "Agent", "Schedule", "Next Run"])
                .style(
                    Style::default()
                        .fg(Theme::MAUVE)
                        .add_modifier(Modifier::BOLD),
                )
                .bottom_margin(1);

            let rows: Vec<Row> = self
                .tasks
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let style = if i == self.selected {
                        Style::default().fg(Theme::TEXT).bg(Theme::SURFACE0)
                    } else {
                        Style::default().fg(Theme::SUBTEXT0)
                    };

                    let icon = if t.enabled { "✅" } else { "⛔" };
                    let icon_style = if t.enabled {
                        Style::default().fg(Theme::GREEN)
                    } else {
                        Style::default().fg(Theme::OVERLAY0)
                    };

                    Row::new(vec![
                        Cell::from(icon).style(icon_style),
                        Cell::from(format!("{}", t.id)).style(style),
                        Cell::from(truncate(&t.name, 25)).style(style),
                        Cell::from(truncate(&t.agent_id, 12)).style(style),
                        Cell::from(Self::format_schedule(t)).style(style),
                        Cell::from(Self::format_next_run(t)).style(style),
                    ])
                })
                .collect();

            let table = Table::new(
                rows,
                [
                    Constraint::Length(3),
                    Constraint::Length(4),
                    Constraint::Percentage(25),
                    Constraint::Percentage(15),
                    Constraint::Percentage(20),
                    Constraint::Percentage(25),
                ],
            )
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Theme::SURFACE1))
                    .title(format!(" Tasks ({}) ", local_timezone_label()))
                    .title_style(Style::default().fg(Theme::MAUVE)),
            );

            frame.render_widget(table, chunks[0]);
        }

        // Help
        let help = Paragraph::new(" Space Toggle  d Delete  r Refresh  ✅ enabled  ⛔ disabled")
            .style(Style::default().fg(Theme::OVERLAY0).bg(Theme::MANTLE));
        frame.render_widget(help, chunks[1]);
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}
