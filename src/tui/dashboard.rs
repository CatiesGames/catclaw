use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;
use serde_json::json;
use tokio::sync::mpsc;
use tracing::error;

use super::theme::Theme;
use super::{Action, Component};
use crate::config::Config;
use crate::logging;
use crate::ws_client::GatewayClient;

// ─── Data types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct AgentInfo {
    id: String,
    is_default: bool,
}

#[derive(Debug, Clone)]
struct SessionInfo {
    agent_id: String,
    origin: String,
    context_id: String,
    state: String,
    last_activity_at: String,
}

#[derive(Debug, Clone)]
struct TaskInfo {
    name: String,
    agent_id: String,
    next_run_at: String,
    enabled: bool,
}

enum DashEvent {
    GatewayStatus { #[allow(dead_code)] agents: usize, active_sessions: usize },
    AgentsLoaded(Vec<AgentInfo>),
    SessionsLoaded(Vec<SessionInfo>),
    TasksLoaded(Vec<TaskInfo>),
}

// ─── Panel struct ─────────────────────────────────────────────────────────────

pub struct DashboardPanel {
    client: Arc<GatewayClient>,
    config: Config,
    log_dir: Option<PathBuf>,
    event_tx: mpsc::UnboundedSender<DashEvent>,
    event_rx: mpsc::UnboundedReceiver<DashEvent>,
    loaded: bool,
    last_refresh: Instant,
    // data
    gateway_active_sessions: usize,
    agents: Vec<AgentInfo>,
    sessions: Vec<SessionInfo>,
    tasks: Vec<TaskInfo>,
    log_records: Vec<logging::LogRecord>,
    last_log_refresh: Instant,
    start_time: Instant,
}

impl DashboardPanel {
    pub fn new(
        client: Arc<GatewayClient>,
        config: &Config,
        log_dir: Option<PathBuf>,
        start_time: Instant,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        DashboardPanel {
            client,
            config: config.clone(),
            log_dir,
            event_tx,
            event_rx,
            loaded: false,
            last_refresh: Instant::now(),
            gateway_active_sessions: 0,
            agents: Vec::new(),
            sessions: Vec::new(),
            tasks: Vec::new(),
            log_records: Vec::new(),
            last_log_refresh: Instant::now(),
            start_time,
        }
    }

    fn refresh(&mut self) {
        self.last_refresh = Instant::now();
        let client = self.client.clone();
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let (status_res, agents_res, sessions_res, tasks_res) = tokio::join!(
                client.request("gateway.status", json!({})),
                client.request("agents.list", json!({})),
                client.request("sessions.list", json!({})),
                client.request("tasks.list", json!({})),
            );
            if let Ok(v) = status_res {
                let active = v
                    .get("active_sessions")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0) as usize;
                let agents = v
                    .get("agents")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0) as usize;
                let _ = tx.send(DashEvent::GatewayStatus {
                    agents,
                    active_sessions: active,
                });
            }
            if let Ok(v) = agents_res {
                let _ = tx.send(DashEvent::AgentsLoaded(parse_agents(&v)));
            } else if let Err(e) = agents_res {
                error!(error = %e, "dashboard: failed to fetch agents");
            }
            if let Ok(v) = sessions_res {
                let _ = tx.send(DashEvent::SessionsLoaded(parse_sessions(&v)));
            } else if let Err(e) = sessions_res {
                error!(error = %e, "dashboard: failed to fetch sessions");
            }
            if let Ok(v) = tasks_res {
                let _ = tx.send(DashEvent::TasksLoaded(parse_tasks(&v)));
            } else if let Err(e) = tasks_res {
                error!(error = %e, "dashboard: failed to fetch tasks");
            }
        });
    }

    fn refresh_logs(&mut self) {
        self.last_log_refresh = Instant::now();
        let Some(log_dir) = &self.log_dir else {
            return;
        };
        let today = logging::today_log_path(log_dir);
        self.log_records = logging::read_log_file(&today);
    }

    fn poll_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                DashEvent::GatewayStatus {
                    active_sessions, ..
                } => {
                    self.gateway_active_sessions = active_sessions;
                }
                DashEvent::AgentsLoaded(agents) => {
                    self.agents = agents;
                }
                DashEvent::SessionsLoaded(sessions) => {
                    self.sessions = sessions;
                }
                DashEvent::TasksLoaded(tasks) => {
                    self.tasks = tasks;
                }
            }
        }
    }
}

// ─── Component impl ───────────────────────────────────────────────────────────

impl Component for DashboardPanel {
    fn handle_event(&mut self, event: &KeyEvent) -> Action {
        match event.code {
            KeyCode::Char('r') => {
                self.refresh();
                self.refresh_logs();
                Action::None
            }
            _ => Action::None,
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Initial load
        if !self.loaded {
            self.loaded = true;
            self.refresh();
            self.refresh_logs();
        }

        // Auto-refresh WS data every 5s
        if self.last_refresh.elapsed().as_secs() >= 5 {
            self.refresh();
        }

        // Auto-refresh logs every 2s
        if self.last_log_refresh.elapsed().as_secs() >= 2 {
            self.refresh_logs();
        }

        self.poll_events();

        // ── Compute dynamic heights ───────────────────────────────────────
        let channel_count = self.config.channels.len().max(1);
        // channel_count rows + 2 border lines
        let channel_height = (channel_count as u16 + 2).min(10);

        let enabled_tasks: Vec<&TaskInfo> = self.tasks.iter().filter(|t| t.enabled).collect();
        let disabled_tasks: Vec<&TaskInfo> = self.tasks.iter().filter(|t| !t.enabled).collect();
        let shown_task_count = (enabled_tasks.len() + disabled_tasks.len()).min(5);
        // rows + header + 2 border lines
        let tasks_height = (shown_task_count as u16 + 3).max(4);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),              // Gateway status
                Constraint::Length(8),              // Agents | Sessions
                Constraint::Length(channel_height), // Channels
                Constraint::Length(tasks_height),   // Tasks
                Constraint::Min(0),                 // Logs
                Constraint::Length(1),              // Help bar
            ])
            .split(area);

        render_gateway_status(
            frame,
            chunks[0],
            &self.config,
            self.start_time,
            self.gateway_active_sessions,
        );
        render_agents_sessions(
            frame,
            chunks[1],
            &self.agents,
            &self.sessions,
            &self.config,
        );
        render_channels(frame, chunks[2], &self.config);
        render_tasks(
            frame,
            chunks[3],
            &enabled_tasks,
            &disabled_tasks,
            shown_task_count,
        );
        render_logs(frame, chunks[4], &self.log_records);

        // Help bar
        let help =
            Paragraph::new(" r Refresh  (auto-refreshes every 5s)")
                .style(Style::default().fg(Theme::OVERLAY0).bg(Theme::MANTLE));
        frame.render_widget(help, chunks[5]);
    }
}

// ─── Section renderers ────────────────────────────────────────────────────────

fn render_gateway_status(
    frame: &mut Frame,
    area: Rect,
    config: &Config,
    start_time: Instant,
    active_sessions: usize,
) {
    let port = config.general.port;
    let pid = crate::pidfile::read_pid(&crate::pidfile::pid_path(Some(config)));
    let uptime_secs = start_time.elapsed().as_secs();
    let uptime_str = format_uptime(uptime_secs);

    let pid_str = pid
        .map(|p| format!("PID {} · ", p))
        .unwrap_or_default();

    let content = Line::from(vec![
        Span::styled("  Connected · ", Style::default().fg(Theme::GREEN)),
        Span::styled(pid_str, Style::default().fg(Theme::OVERLAY0)),
        Span::styled(
            format!("port {} · ", port),
            Style::default().fg(Theme::OVERLAY0),
        ),
        Span::styled(
            format!("uptime {} · ", uptime_str),
            Style::default().fg(Theme::OVERLAY0),
        ),
        Span::styled(
            format!("{} active session(s)", active_sessions),
            Style::default().fg(Theme::SUBTEXT0),
        ),
        Span::styled(format!("  v{}", env!("CARGO_PKG_VERSION")), Style::default().fg(Theme::SURFACE2)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::SURFACE1))
        .title(" Gateway ")
        .title_style(Style::default().fg(Theme::GREEN));

    let para = Paragraph::new(content).block(block);
    frame.render_widget(para, area);
}

fn render_agents_sessions(
    frame: &mut Frame,
    area: Rect,
    agents: &[AgentInfo],
    sessions: &[SessionInfo],
    config: &Config,
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    // ── Agents ─────────────────────────────────────────────────────────────
    let agent_title = format!(" Agents ({}) ", agents.len());
    let agent_rows: Vec<Line> = if agents.is_empty() {
        vec![Line::from(Span::styled(
            "  (none)",
            Style::default().fg(Theme::OVERLAY0),
        ))]
    } else {
        agents
            .iter()
            .map(|a| {
                if a.is_default {
                    Line::from(vec![
                        Span::styled("  ⭐ ", Style::default().fg(Theme::YELLOW)),
                        Span::styled(
                            a.id.clone(),
                            Style::default()
                                .fg(Theme::MAUVE)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ])
                } else {
                    Line::from(vec![
                        Span::styled("     ", Style::default()),
                        Span::styled(a.id.clone(), Style::default().fg(Theme::SUBTEXT0)),
                    ])
                }
            })
            .collect()
    };

    let agents_block = Paragraph::new(agent_rows)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Theme::SURFACE1))
                .title(agent_title)
                .title_style(Style::default().fg(Theme::MAUVE)),
        );
    frame.render_widget(agents_block, cols[0]);

    // ── Sessions ────────────────────────────────────────────────────────────
    let max_concurrent = config.general.max_concurrent_sessions;
    let active_count = sessions.iter().filter(|s| s.state == "active").count();
    let idle_count = sessions.iter().filter(|s| s.state == "idle").count();
    let session_title = format!(
        " Sessions ({} running · {} idle · {} concurrent limit) ",
        active_count, idle_count, max_concurrent
    );

    let session_rows: Vec<Line> = if sessions.is_empty() {
        vec![Line::from(Span::styled(
            "  (no sessions)",
            Style::default().fg(Theme::OVERLAY0),
        ))]
    } else {
        sessions
            .iter()
            .map(|s| {
                let (icon, icon_color) = state_icon(&s.state);
                let time_str = format_relative_time(&s.last_activity_at);
                let agent_w = 8usize;
                let origin_w = 10usize;
                let ctx_w = 18usize;
                Line::from(vec![
                    Span::styled(
                        format!("  {} ", icon),
                        Style::default().fg(icon_color),
                    ),
                    Span::styled(
                        pad_right(&s.agent_id, agent_w),
                        Style::default().fg(Theme::MAUVE),
                    ),
                    Span::styled(
                        format!("· {} ", pad_right(&s.origin, origin_w)),
                        Style::default().fg(Theme::OVERLAY1),
                    ),
                    Span::styled(
                        pad_right(&s.context_id, ctx_w),
                        Style::default().fg(Theme::SUBTEXT0),
                    ),
                    Span::styled(
                        format!(" {}", time_str),
                        Style::default().fg(Theme::OVERLAY0),
                    ),
                ])
            })
            .collect()
    };

    let sessions_block = Paragraph::new(session_rows)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Theme::SURFACE1))
                .title(session_title)
                .title_style(Style::default().fg(Theme::SAPPHIRE)),
        );
    frame.render_widget(sessions_block, cols[1]);
}

fn render_channels(frame: &mut Frame, area: Rect, config: &Config) {
    let channel_count = config.channels.len();
    let title = format!(" Channels ({}) ", channel_count);

    let rows: Vec<Line> = if config.channels.is_empty() {
        vec![Line::from(Span::styled(
            "  (no channels configured)",
            Style::default().fg(Theme::OVERLAY0),
        ))]
    } else {
        config
            .channels
            .iter()
            .map(|ch| {
                Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(
                        pad_right(&ch.channel_type, 10),
                        Style::default().fg(Theme::SAPPHIRE),
                    ),
                    Span::styled(
                        format!("{} ", pad_right(&ch.activation, 8)),
                        Style::default().fg(Theme::PEACH),
                    ),
                    Span::styled(
                        format!("dm:{} ", ch.dm_policy),
                        Style::default().fg(Theme::OVERLAY1),
                    ),
                    Span::styled(
                        format!("group:{}", ch.group_policy),
                        Style::default().fg(Theme::OVERLAY1),
                    ),
                ])
            })
            .collect()
    };

    let block = Paragraph::new(rows).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::SURFACE1))
            .title(title)
            .title_style(Style::default().fg(Theme::LAVENDER)),
    );
    frame.render_widget(block, area);
}

fn render_tasks(
    frame: &mut Frame,
    area: Rect,
    enabled_tasks: &[&TaskInfo],
    disabled_tasks: &[&TaskInfo],
    shown_count: usize,
) {
    let all_tasks: Vec<&TaskInfo> = enabled_tasks
        .iter()
        .chain(disabled_tasks.iter())
        .copied()
        .take(shown_count)
        .collect();

    let title = format!(
        " Upcoming Tasks ({} enabled) ",
        enabled_tasks.len()
    );

    let rows: Vec<Line> = if all_tasks.is_empty() {
        vec![Line::from(Span::styled(
            "  (no scheduled tasks)",
            Style::default().fg(Theme::OVERLAY0),
        ))]
    } else {
        all_tasks
            .iter()
            .map(|t| {
                let (icon, icon_color) = if t.enabled {
                    ("✅", Theme::GREEN)
                } else {
                    ("⛔", Theme::OVERLAY0)
                };
                let next_str = if t.enabled {
                    format_next_run_relative(&t.next_run_at)
                } else {
                    "(disabled)".to_string()
                };
                Line::from(vec![
                    Span::styled(
                        format!("  {} ", icon),
                        Style::default().fg(icon_color),
                    ),
                    Span::styled(
                        pad_right(&t.name, 20),
                        Style::default().fg(Theme::TEXT),
                    ),
                    Span::styled(
                        format!(" {} ", pad_right(&t.agent_id, 10)),
                        Style::default().fg(Theme::SUBTEXT0),
                    ),
                    Span::styled(next_str, Style::default().fg(Theme::PEACH)),
                ])
            })
            .collect()
    };

    let block = Paragraph::new(rows).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::SURFACE1))
            .title(title)
            .title_style(Style::default().fg(Theme::YELLOW)),
    );
    frame.render_widget(block, area);
}

fn render_logs(frame: &mut Frame, area: Rect, records: &[logging::LogRecord]) {
    // Show last 8 records at INFO level or above
    let filtered: Vec<&logging::LogRecord> = records
        .iter()
        .filter(|r| matches!(r.level.to_uppercase().as_str(), "ERROR" | "WARN" | "INFO"))
        .collect();
    let last_8: Vec<&logging::LogRecord> = filtered
        .iter()
        .rev()
        .take(8)
        .rev()
        .copied()
        .collect();

    let lines: Vec<Line> = last_8
        .iter()
        .map(|r| {
            let time = extract_time(&r.ts);
            let (level_str, level_color) = match r.level.to_uppercase().as_str() {
                "ERROR" => ("ERROR", Theme::RED),
                "WARN" => ("WARN ", Theme::YELLOW),
                "INFO" => ("INFO ", Theme::GREEN),
                _ => ("DEBUG", Theme::OVERLAY1),
            };
            let target_short = if let Some(stripped) = r.target.strip_prefix("catclaw::") {
                format!("[{}] ", stripped)
            } else if r.target.is_empty() {
                String::new()
            } else {
                format!("[{}] ", r.target)
            };
            Line::from(vec![
                Span::styled(
                    format!(" {} ", time),
                    Style::default().fg(Theme::OVERLAY0),
                ),
                Span::styled(
                    format!("{} ", level_str),
                    Style::default().fg(level_color),
                ),
                Span::styled(target_short, Style::default().fg(Theme::SURFACE2)),
                Span::styled(r.msg.clone(), Style::default().fg(Theme::TEXT)),
            ])
        })
        .collect();

    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::SURFACE1))
            .title(" Recent Logs (INFO+) ")
            .title_style(Style::default().fg(Theme::MAUVE)),
    );
    frame.render_widget(para, area);
}

// ─── Parse helpers ────────────────────────────────────────────────────────────

fn parse_agents(val: &serde_json::Value) -> Vec<AgentInfo> {
    let arr = match val.as_array() {
        Some(a) => a,
        None => return vec![],
    };
    arr.iter()
        .filter_map(|v| {
            Some(AgentInfo {
                id: v.get("id")?.as_str()?.to_string(),
                is_default: v
                    .get("default")
                    .and_then(|d| d.as_bool())
                    .unwrap_or(false),
            })
        })
        .collect()
}

fn parse_sessions(val: &serde_json::Value) -> Vec<SessionInfo> {
    let arr = match val.as_array() {
        Some(a) => a,
        None => return vec![],
    };
    arr.iter()
        .filter_map(|v| {
            Some(SessionInfo {
                agent_id: v.get("agent_id")?.as_str()?.to_string(),
                origin: v
                    .get("origin")
                    .and_then(|x| x.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                context_id: v
                    .get("context_id")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                state: v
                    .get("state")
                    .and_then(|x| x.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                last_activity_at: v
                    .get("last_activity_at")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

fn parse_tasks(val: &serde_json::Value) -> Vec<TaskInfo> {
    let arr = match val.as_array() {
        Some(a) => a,
        None => return vec![],
    };
    let mut tasks: Vec<TaskInfo> = arr
        .iter()
        .filter_map(|v| {
            Some(TaskInfo {
                name: v.get("name")?.as_str()?.to_string(),
                agent_id: v.get("agent_id")?.as_str()?.to_string(),
                next_run_at: v
                    .get("next_run_at")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                enabled: v
                    .get("enabled")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false),
            })
        })
        .collect();
    // Sort: enabled first, then by next_run_at
    tasks.sort_by(|a, b| {
        b.enabled
            .cmp(&a.enabled)
            .then_with(|| a.next_run_at.cmp(&b.next_run_at))
    });
    tasks
}

// ─── Formatting helpers ───────────────────────────────────────────────────────

fn format_uptime(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{}s", secs)
    }
}

/// Format a past RFC3339 timestamp as a relative string.
fn format_relative_time(ts: &str) -> String {
    if ts.is_empty() {
        return String::new();
    }
    let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) else {
        return ts[..ts.len().min(16)].replace('T', " ");
    };
    let diff = chrono::Utc::now().signed_duration_since(dt.with_timezone(&chrono::Utc));
    let secs = diff.num_seconds();
    if secs < 0 {
        return "just now".to_string();
    }
    if secs < 60 {
        return "just now".to_string();
    }
    if secs < 3600 {
        return format!("{}m ago", secs / 60);
    }
    if secs < 86400 {
        return format!("{}h ago", secs / 3600);
    }
    // Older: show date
    dt.format("%Y-%m-%d").to_string()
}

/// Format a future RFC3339 timestamp as a relative "in X" string.
fn format_next_run_relative(ts: &str) -> String {
    if ts.is_empty() {
        return "(unknown)".to_string();
    }
    let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) else {
        if ts.len() >= 16 {
            return ts[..16].replace('T', " ");
        }
        return ts.to_string();
    };
    let diff = dt
        .with_timezone(&chrono::Utc)
        .signed_duration_since(chrono::Utc::now());
    let secs = diff.num_seconds();
    if secs <= 0 {
        return "now".to_string();
    }
    if secs < 3600 {
        return format!("in {}m", secs / 60);
    }
    if secs < 86400 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m == 0 {
            return format!("in {}h", h);
        }
        return format!("in {}h {}m", h, m);
    }
    dt.with_timezone(&chrono::Local).format("%m-%d %H:%M").to_string()
}

fn state_icon(state: &str) -> (&'static str, Color) {
    match state {
        "active" => ("●", Theme::GREEN),
        "idle" => ("●", Theme::YELLOW),
        "suspended" => ("○", Theme::OVERLAY0),
        "pending" | "new" => ("◌", Theme::OVERLAY0),
        _ => ("·", Theme::OVERLAY0),
    }
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

fn pad_right(s: &str, width: usize) -> String {
    if s.len() >= width {
        s[..width].to_string()
    } else {
        format!("{:<width$}", s, width = width)
    }
}
