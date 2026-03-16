mod agents;
mod bindings;
mod chat;
mod config_panel;
mod dashboard;
mod editor;
mod logs;
mod sessions;
mod skills;
pub(crate) mod splash;
mod tasks;
mod theme;

use std::io;
use std::time::Instant;

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers, EnableMouseCapture, DisableMouseCapture, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use ratatui::widgets::*;

use unicode_width::UnicodeWidthStr;

use std::sync::Arc;

use crate::config::Config;
use crate::error::Result;
use crate::ws_client::GatewayClient;

use theme::Theme;

/// Actions that components can emit
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Action {
    Quit,
    SwitchTab(Tab),
    Refresh,
    SetLogLevel(String),
    None,
}

/// Available tabs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Sessions,
    Agents,
    Skills,
    Tasks,
    Bindings,
    Config,
    Logs,
}

impl Tab {
    pub fn all() -> &'static [Tab] {
        &[
            Tab::Dashboard,
            Tab::Sessions,
            Tab::Agents,
            Tab::Skills,
            Tab::Tasks,
            Tab::Bindings,
            Tab::Config,
            Tab::Logs,
        ]
    }

    pub fn label(&self) -> &str {
        match self {
            Tab::Dashboard => "Dashboard",
            Tab::Sessions => "Sessions",
            Tab::Agents => "Agents",
            Tab::Skills => "Skills",
            Tab::Tasks => "Tasks",
            Tab::Bindings => "Bindings",
            Tab::Config => "Config",
            Tab::Logs => "Logs",
        }
    }

    pub fn icon(&self) -> &str {
        match self {
            Tab::Dashboard => "🏠",
            Tab::Sessions => "💬",
            Tab::Agents => "🤖",
            Tab::Skills => "⚡",
            Tab::Tasks => "📋",
            Tab::Bindings => "🔗",
            Tab::Config => "⚙️",
            Tab::Logs => "📜",
        }
    }

    pub fn next(&self) -> Tab {
        let tabs = Tab::all();
        let idx = tabs.iter().position(|t| t == self).unwrap_or(0);
        tabs[(idx + 1) % tabs.len()]
    }

    pub fn prev(&self) -> Tab {
        let tabs = Tab::all();
        let idx = tabs.iter().position(|t| t == self).unwrap_or(0);
        tabs[(idx + tabs.len() - 1) % tabs.len()]
    }
}

/// Component trait for TUI panels
pub trait Component {
    fn handle_event(&mut self, event: &KeyEvent) -> Action;
    fn render(&mut self, frame: &mut Frame, area: Rect);

    /// Whether the component is in a mode that captures all input
    /// (e.g., text editing). When true, global keybindings are suppressed.
    fn captures_input(&self) -> bool {
        false
    }
}

/// App phase
#[derive(Debug, Clone, PartialEq)]
enum Phase {
    /// Animated splash screen
    Splash,
    /// Main TUI with tabs
    Main,
}

/// Main TUI application
struct App {
    phase: Phase,
    splash_start: Instant,
    start_time: Instant,
    active_tab: Tab,
    dashboard_panel: dashboard::DashboardPanel,
    sessions_panel: sessions::SessionsPanel,
    agents_panel: agents::AgentsPanel,
    skills_panel: skills::SkillsPanel,
    tasks_panel: tasks::TasksPanel,
    bindings_panel: bindings::BindingsPanel,
    config_panel: config_panel::ConfigPanel,
    logs_panel: logs::LogsPanel,
    should_quit: bool,
    agent_count: usize,
    channel_count: usize,
}

impl App {
    fn new(
        config: &Config,
        client: Arc<GatewayClient>,
        ws_event_rx: tokio::sync::mpsc::UnboundedReceiver<crate::ws_protocol::WsEvent>,
        config_path: std::path::PathBuf,
    ) -> Self {
        let now = Instant::now();

        let log_dir = config.logging.resolve_log_dir(&config.general.workspace);

        App {
            phase: Phase::Splash,
            splash_start: now,
            start_time: now,
            active_tab: Tab::Dashboard,
            dashboard_panel: dashboard::DashboardPanel::new(
                client.clone(),
                config,
                Some(log_dir.clone()),
                now,
            ),
            sessions_panel: sessions::SessionsPanel::new(
                client.clone(),
                ws_event_rx,
                config.general.streaming,
                config,
            ),
            agents_panel: agents::AgentsPanel::new(config, config_path.clone(), client.clone()),
            skills_panel: skills::SkillsPanel::new(config),
            tasks_panel: tasks::TasksPanel::new(client.clone()),
            bindings_panel: bindings::BindingsPanel::new(config, config_path.clone()),
            config_panel: config_panel::ConfigPanel::new(config, config_path, client.clone()),
            logs_panel: {
                let mut panel = logs::LogsPanel::new(Some(log_dir), &config.logging.level);
                panel.add_log(logs::LogLevel::Info, "CatClaw TUI started".to_string());
                panel
            },
            should_quit: false,
            agent_count: config.agents.len(),
            channel_count: config.channels.len(),
        }
    }

    fn handle_event(&mut self, event: &KeyEvent) -> Action {
        // During splash, any key dismisses it
        if self.phase == Phase::Splash {
            self.phase = Phase::Main;
            return Action::Refresh;
        }

        // Check if the active panel captures all input (e.g., text editor mode)
        let panel_captures = match self.active_tab {
            Tab::Dashboard => self.dashboard_panel.captures_input(),
            Tab::Sessions => self.sessions_panel.captures_input(),
            Tab::Agents => self.agents_panel.captures_input(),
            Tab::Skills => self.skills_panel.captures_input(),
            Tab::Tasks => self.tasks_panel.captures_input(),
            Tab::Bindings => self.bindings_panel.captures_input(),
            Tab::Config => self.config_panel.captures_input(),
            Tab::Logs => self.logs_panel.captures_input(),
        };

        // Ctrl+C always quits
        if event.modifiers == KeyModifiers::CONTROL && event.code == KeyCode::Char('c') {
            self.should_quit = true;
            return Action::Quit;
        }

        // Global keybindings (suppressed when panel captures input)
        if !panel_captures {
            match (event.modifiers, event.code) {
                (_, KeyCode::Char('q')) => {
                    self.should_quit = true;
                    return Action::Quit;
                }
                (_, KeyCode::Tab) if event.modifiers.is_empty() => {
                    self.active_tab = self.active_tab.next();
                    return Action::Refresh;
                }
                (_, KeyCode::BackTab) => {
                    self.active_tab = self.active_tab.prev();
                    return Action::Refresh;
                }
                // Number keys for quick tab switch
                (_, KeyCode::Char('1')) if event.modifiers.contains(KeyModifiers::ALT) => {
                    self.active_tab = Tab::Dashboard;
                    return Action::Refresh;
                }
                (_, KeyCode::Char('2')) if event.modifiers.contains(KeyModifiers::ALT) => {
                    self.active_tab = Tab::Sessions;
                    return Action::Refresh;
                }
                (_, KeyCode::Char('3')) if event.modifiers.contains(KeyModifiers::ALT) => {
                    self.active_tab = Tab::Agents;
                    return Action::Refresh;
                }
                (_, KeyCode::Char('4')) if event.modifiers.contains(KeyModifiers::ALT) => {
                    self.active_tab = Tab::Skills;
                    return Action::Refresh;
                }
                (_, KeyCode::Char('5')) if event.modifiers.contains(KeyModifiers::ALT) => {
                    self.active_tab = Tab::Tasks;
                    return Action::Refresh;
                }
                (_, KeyCode::Char('6')) if event.modifiers.contains(KeyModifiers::ALT) => {
                    self.active_tab = Tab::Bindings;
                    return Action::Refresh;
                }
                (_, KeyCode::Char('7')) if event.modifiers.contains(KeyModifiers::ALT) => {
                    self.active_tab = Tab::Config;
                    return Action::Refresh;
                }
                (_, KeyCode::Char('8')) if event.modifiers.contains(KeyModifiers::ALT) => {
                    self.active_tab = Tab::Logs;
                    return Action::Refresh;
                }
                _ => {}
            }
        } else if matches!(
            (event.modifiers, event.code),
            (KeyModifiers::CONTROL, KeyCode::Char('c'))
        ) {
            return Action::Quit;
        }

        // Delegate to active panel
        let action = match self.active_tab {
            Tab::Dashboard => self.dashboard_panel.handle_event(event),
            Tab::Sessions => self.sessions_panel.handle_event(event),
            Tab::Agents => self.agents_panel.handle_event(event),
            Tab::Skills => self.skills_panel.handle_event(event),
            Tab::Tasks => self.tasks_panel.handle_event(event),
            Tab::Bindings => self.bindings_panel.handle_event(event),
            Tab::Config => self.config_panel.handle_event(event),
            Tab::Logs => self.logs_panel.handle_event(event),
        };

        match &action {
            Action::Quit => self.should_quit = true,
            Action::SwitchTab(tab) => self.active_tab = *tab,
            Action::SetLogLevel(level) => self.logs_panel.set_level(level),
            _ => {}
        }

        action
    }

    fn handle_mouse(&mut self, mouse: &crossterm::event::MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                match self.active_tab {
                    Tab::Sessions => self.sessions_panel.scroll_up(3),
                    Tab::Logs => self.logs_panel.scroll_up(3),
                    _ => {}
                }
            }
            MouseEventKind::ScrollDown => {
                match self.active_tab {
                    Tab::Sessions => self.sessions_panel.scroll_down(3),
                    Tab::Logs => self.logs_panel.scroll_down(3),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Called every tick to process background events regardless of active tab.
    fn tick(&mut self) {
        self.sessions_panel.poll_responses();
    }

    fn render(&mut self, frame: &mut Frame) {
        // Full background
        let bg = Block::default().style(Style::default().bg(Theme::BASE));
        frame.render_widget(bg, frame.area());

        match self.phase {
            Phase::Splash => {
                let elapsed = self.splash_start.elapsed().as_millis() as u16;
                let tick = (elapsed / 100).min(20); // ~100ms per tick
                splash::render_splash(frame, frame.area(), tick);
            }
            Phase::Main => {
                self.render_main(frame);
            }
        }
    }

    fn render_main(&mut self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Tab bar
                Constraint::Min(0),   // Content
                Constraint::Length(1), // Status bar
            ])
            .split(frame.area());

        // Tab bar
        self.render_tabs(frame, chunks[0]);

        // Active panel
        match self.active_tab {
            Tab::Dashboard => self.dashboard_panel.render(frame, chunks[1]),
            Tab::Sessions => self.sessions_panel.render(frame, chunks[1]),
            Tab::Agents => self.agents_panel.render(frame, chunks[1]),
            Tab::Skills => self.skills_panel.render(frame, chunks[1]),
            Tab::Tasks => self.tasks_panel.render(frame, chunks[1]),
            Tab::Bindings => self.bindings_panel.render(frame, chunks[1]),
            Tab::Config => self.config_panel.render(frame, chunks[1]),
            Tab::Logs => self.logs_panel.render(frame, chunks[1]),
        }

        // Status bar
        self.render_status_bar(frame, chunks[2]);
    }

    fn render_tabs(&self, frame: &mut Frame, area: Rect) {
        let titles: Vec<Line> = Tab::all()
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let is_active = *tab == self.active_tab;
                let num = format!("{}", i + 1);

                if is_active {
                    Line::from(vec![
                        Span::styled(
                            format!("{} ", tab.icon()),
                            Style::default().fg(Theme::MAUVE),
                        ),
                        Span::styled(
                            tab.label(),
                            Style::default()
                                .fg(Theme::MAUVE)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ])
                } else {
                    Line::from(vec![
                        Span::styled(
                            format!("{} ", num),
                            Style::default().fg(Theme::SURFACE2),
                        ),
                        Span::styled(tab.label(), Style::default().fg(Theme::OVERLAY1)),
                    ])
                }
            })
            .collect();

        let tabs = Tabs::new(titles)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Theme::SURFACE1))
                    .title(Span::styled(
                        " 🐱 CatClaw ",
                        Style::default()
                            .fg(Theme::MAUVE)
                            .add_modifier(Modifier::BOLD),
                    )),
            )
            .select(
                Tab::all()
                    .iter()
                    .position(|t| t == &self.active_tab)
                    .unwrap_or(0),
            )
            .highlight_style(
                Style::default()
                    .fg(Theme::MAUVE)
                    .add_modifier(Modifier::BOLD),
            )
            .divider(Span::styled(" │ ", Style::default().fg(Theme::SURFACE1)));

        frame.render_widget(tabs, area);
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect) {
        // Full background
        let bg = Block::default().style(Style::default().bg(Theme::MANTLE));
        frame.render_widget(bg, area);

        // Left: keyboard hints
        let left = vec![
            Span::styled(" Tab", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD).bg(Theme::MANTLE)),
            Span::styled(" Navigate  ", Style::default().fg(Theme::OVERLAY0).bg(Theme::MANTLE)),
            Span::styled("⌥N", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD).bg(Theme::MANTLE)),
            Span::styled(" Jump  ", Style::default().fg(Theme::OVERLAY0).bg(Theme::MANTLE)),
            Span::styled("q", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD).bg(Theme::MANTLE)),
            Span::styled(" Quit", Style::default().fg(Theme::OVERLAY0).bg(Theme::MANTLE)),
        ];
        frame.render_widget(Paragraph::new(Line::from(left)), area);

        // Right: system metrics + version
        let session_stats = self.sessions_panel.stats();
        let uptime_secs = self.start_time.elapsed().as_secs();
        let uptime_str = if uptime_secs >= 3600 {
            format!("{}h{}m", uptime_secs / 3600, (uptime_secs % 3600) / 60)
        } else if uptime_secs >= 60 {
            format!("{}m{}s", uptime_secs / 60, uptime_secs % 60)
        } else {
            format!("{}s", uptime_secs)
        };

        let right_spans = vec![
            Span::styled(
                "● ",
                Style::default().fg(Theme::GREEN).bg(Theme::MANTLE),
            ),
            Span::styled(
                "Gateway ",
                Style::default().fg(Theme::GREEN).bg(Theme::MANTLE),
            ),
            Span::styled("│ ", Style::default().fg(Theme::SURFACE1).bg(Theme::MANTLE)),
            Span::styled("📡 ", Style::default().fg(Theme::GREEN).bg(Theme::MANTLE)),
            Span::styled(
                format!("{}active ", session_stats.0),
                Style::default().fg(Theme::GREEN).bg(Theme::MANTLE),
            ),
            Span::styled(
                format!("{}idle ", session_stats.1),
                Style::default().fg(Theme::YELLOW).bg(Theme::MANTLE),
            ),
            Span::styled("│ ", Style::default().fg(Theme::SURFACE1).bg(Theme::MANTLE)),
            Span::styled(
                format!("🤖{} ", self.agent_count),
                Style::default().fg(Theme::LAVENDER).bg(Theme::MANTLE),
            ),
            Span::styled(
                format!("💬{} ", self.channel_count),
                Style::default().fg(Theme::SAPPHIRE).bg(Theme::MANTLE),
            ),
            Span::styled("│ ", Style::default().fg(Theme::SURFACE1).bg(Theme::MANTLE)),
            Span::styled(
                format!("⏱️{} ", uptime_str),
                Style::default().fg(Theme::OVERLAY0).bg(Theme::MANTLE),
            ),
            Span::styled("│ ", Style::default().fg(Theme::SURFACE1).bg(Theme::MANTLE)),
            Span::styled(
                "v0.1.0 ",
                Style::default().fg(Theme::SURFACE2).bg(Theme::MANTLE),
            ),
        ];

        let right_width: u16 = right_spans
            .iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()) as u16)
            .sum();
        if area.width > right_width {
            let right_area = Rect::new(
                area.x + area.width - right_width,
                area.y,
                right_width,
                1,
            );
            frame.render_widget(Paragraph::new(Line::from(right_spans)), right_area);
        }
    }
}

/// Restore terminal state (called on clean exit and panic)
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
    // Show cursor
    let _ = execute!(io::stdout(), crossterm::cursor::Show);
}

/// Run the TUI, connecting to a Gateway via WebSocket.
pub async fn run(
    config: Config,
    config_path: std::path::PathBuf,
    ws_url: &str,
) -> Result<()> {
    // Connect to Gateway WebSocket
    let (client, ws_event_rx) = GatewayClient::connect(ws_url, &config.general.ws_token)
        .await
        .map_err(|e| crate::error::CatClawError::Other(format!("WS connect failed: {}", e)))?;

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Install panic hook that restores terminal before printing panic
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_panic(info);
    }));

    // Create app
    let mut app = App::new(&config, client, ws_event_rx, config_path);

    // Main loop
    loop {
        // Process background events every tick (streaming, WS events)
        app.tick();

        terminal.draw(|frame| {
            app.render(frame);
        })?;

        // Auto-dismiss splash after 2 seconds
        if app.phase == Phase::Splash && app.splash_start.elapsed().as_secs() >= 2 {
            app.phase = Phase::Main;
        }

        if event::poll(std::time::Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    app.handle_event(&key);
                    if app.should_quit {
                        break;
                    }
                }
                Event::Mouse(mouse) => {
                    app.handle_mouse(&mouse);
                }
                _ => {}
            }
        }
    }

    // Restore terminal
    restore_terminal();

    // Restore default panic hook
    let _ = std::panic::take_hook();

    Ok(())
}
