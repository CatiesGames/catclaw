use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;

use super::theme::Theme;
use super::{Action, Component};
use crate::agent::{AgentLoader, SkillInfo, SkillSource};
use crate::config::Config;

/// A skill from the shared pool with per-agent enable status
struct SkillEntry {
    info: SkillInfo,
    /// Which agents have it enabled (for display)
    enabled_agents: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum InputMode {
    Normal,
    /// Typing a skill source to install
    Install,
    /// Confirming uninstall from shared pool
    ConfirmUninstall,
}

pub struct SkillsPanel {
    config: Config,
    /// Skills from shared pool, deduplicated
    skills: Vec<SkillEntry>,
    selected: usize,
    mode: InputMode,
    input_buf: String,
    status_msg: Option<String>,
}

impl SkillsPanel {
    pub fn new(config: &Config) -> Self {
        let skills = Self::load_skills(config);
        SkillsPanel {
            config: config.clone(),
            skills,
            selected: 0,
            mode: InputMode::Normal,
            input_buf: String::new(),
            status_msg: None,
        }
    }

    fn load_skills(config: &Config) -> Vec<SkillEntry> {
        let workspace_root = &config.general.workspace;
        let shared_dir = workspace_root.join("skills");
        if !shared_dir.exists() {
            return Vec::new();
        }

        // Read all skills from shared pool
        let mut all_names: Vec<String> = std::fs::read_dir(&shared_dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir() && e.path().join("SKILL.md").exists())
            .filter_map(|e| e.file_name().to_str().map(String::from))
            .collect();
        all_names.sort();

        all_names.into_iter().map(|name| {
            // For each agent, check if this skill is enabled
            let enabled_agents: Vec<String> = config.agents.iter()
                .filter(|agent| {
                    let skills = AgentLoader::list_skills(&agent.workspace, workspace_root);
                    skills.iter().any(|s| s.name == name && s.is_enabled)
                })
                .map(|a| a.id.clone())
                .collect();

            // Read description from shared pool
            let skill_md = shared_dir.join(&name).join("SKILL.md");
            let description = read_skill_desc(&skill_md);

            SkillEntry {
                info: SkillInfo { name, is_enabled: !enabled_agents.is_empty(), description },
                enabled_agents,
            }
        }).collect()
    }

    fn refresh(&mut self) {
        self.skills = Self::load_skills(&self.config);
        if self.selected >= self.skills.len() && !self.skills.is_empty() {
            self.selected = self.skills.len() - 1;
        }
    }

    fn start_install(&mut self) {
        self.mode = InputMode::Install;
        self.input_buf.clear();
        self.status_msg = None;
    }

    fn confirm_install(&mut self) {
        let source_str = self.input_buf.trim().to_string();
        self.mode = InputMode::Normal;
        if source_str.is_empty() {
            self.status_msg = Some("Cancelled".into());
            return;
        }
        let source = match SkillSource::parse(&source_str) {
            Ok(s) => s,
            Err(e) => { self.status_msg = Some(format!("Error: {}", e)); return; }
        };
        let workspace_root = self.config.general.workspace.clone();
        let rt = tokio::runtime::Handle::current();
        match rt.block_on(AgentLoader::install_skill(&workspace_root, &source)) {
            Ok(()) => {
                let name = match &source {
                    SkillSource::Anthropic(n) => n.clone(),
                    SkillSource::GitHub { path, .. } => path.rsplit('/').next().unwrap_or(path).to_string(),
                    SkillSource::Local(p) => p.file_name().and_then(|n| n.to_str()).unwrap_or("unknown").to_string(),
                };
                self.status_msg = Some(format!("'{}' installed to shared pool", name));
                self.refresh();
            }
            Err(e) => { self.status_msg = Some(format!("Install failed: {}", e)); }
        }
    }

    fn start_uninstall(&mut self) {
        if self.skills.get(self.selected).is_some() {
            self.mode = InputMode::ConfirmUninstall;
            self.status_msg = None;
        }
    }

    fn confirm_uninstall(&mut self) {
        if let Some(entry) = self.skills.get(self.selected) {
            let name = entry.info.name.clone();
            match AgentLoader::uninstall_skill(&self.config.general.workspace, &name) {
                Ok(()) => {
                    self.status_msg = Some(format!("'{}' removed from shared pool", name));
                    self.refresh();
                }
                Err(e) => { self.status_msg = Some(format!("Uninstall failed: {}", e)); }
            }
        }
        self.mode = InputMode::Normal;
    }
}

fn read_skill_desc(skill_md: &std::path::Path) -> String {
    let content = match std::fs::read_to_string(skill_md) { Ok(c) => c, Err(_) => return String::new() };
    let body = content.strip_prefix("---").unwrap_or(&content);
    let end = body.find("\n---").unwrap_or(body.len());
    for line in body[..end].lines() {
        if let Some(rest) = line.strip_prefix("description:") {
            return rest.trim().trim_matches('"').to_string();
        }
    }
    String::new()
}

impl Component for SkillsPanel {
    fn handle_event(&mut self, event: &KeyEvent) -> Action {
        match &self.mode {
            InputMode::Normal => match event.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    if !self.skills.is_empty() {
                        self.selected = (self.selected + 1).min(self.skills.len() - 1);
                    }
                    Action::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    Action::None
                }
                KeyCode::Char('i') => { self.start_install(); Action::None }
                KeyCode::Char('x') | KeyCode::Char('d') => { self.start_uninstall(); Action::None }
                KeyCode::Char('r') => { self.refresh(); Action::None }
                _ => Action::None,
            },
            InputMode::Install => {
                match event.code {
                    KeyCode::Enter => { self.confirm_install(); }
                    KeyCode::Esc => {
                        self.mode = InputMode::Normal;
                        self.status_msg = Some("Cancelled".into());
                    }
                    KeyCode::Backspace => { self.input_buf.pop(); }
                    KeyCode::Char(c) => { self.input_buf.push(c); }
                    _ => {}
                }
                Action::None
            }
            InputMode::ConfirmUninstall => match event.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => { self.confirm_uninstall(); Action::None }
                _ => {
                    self.mode = InputMode::Normal;
                    self.status_msg = Some("Cancelled".into());
                    Action::None
                }
            },
        }
    }

    fn captures_input(&self) -> bool {
        !matches!(self.mode, InputMode::Normal)
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        let header = Row::new(vec!["Skill", "Description", "Enabled for"])
            .style(Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD))
            .bottom_margin(1);

        let rows: Vec<Row> = self.skills.iter().enumerate().map(|(i, entry)| {
            let is_sel = i == self.selected;
            let style = if is_sel {
                Style::default().fg(Theme::TEXT).bg(Theme::SURFACE0)
            } else {
                Style::default().fg(Theme::SUBTEXT0)
            };
            let agents_str = if entry.enabled_agents.is_empty() {
                "none".to_string()
            } else {
                entry.enabled_agents.join(", ")
            };
            Row::new(vec![
                Cell::from(entry.info.name.clone()).style(
                    style.add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() })
                ),
                Cell::from(entry.info.description.clone()).style(Style::default().fg(Theme::OVERLAY1)),
                Cell::from(agents_str).style(Style::default().fg(Theme::SAPPHIRE)),
            ])
        }).collect();

        let table = Table::new(
            rows,
            [Constraint::Length(20), Constraint::Min(0), Constraint::Length(18)],
        )
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Theme::SURFACE1))
                .title(format!(" ⚡ Shared Skills ({}) ", self.skills.len()))
                .title_style(Style::default().fg(Theme::MAUVE)),
        );

        frame.render_widget(table, chunks[0]);

        let status_line = match &self.mode {
            InputMode::Install => Paragraph::new(Line::from(vec![
                Span::styled(" Install: ", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{}▌", self.input_buf), Style::default().fg(Theme::TEXT)),
                Span::styled("  @anthropic/<name>  github:owner/repo/path  /local/path", Style::default().fg(Theme::OVERLAY0)),
            ])).style(Style::default().bg(Theme::SURFACE0)),
            InputMode::ConfirmUninstall => {
                let name = self.skills.get(self.selected).map(|s| s.info.name.as_str()).unwrap_or("?");
                Paragraph::new(format!(" Remove '{}' from shared pool? (y/N)", name))
                    .style(Style::default().fg(Theme::RED).bg(Theme::SURFACE0))
            }
            InputMode::Normal => {
                if let Some(msg) = &self.status_msg {
                    Paragraph::new(format!(" {}", msg)).style(Style::default().fg(Theme::GREEN).bg(Theme::MANTLE))
                } else {
                    Paragraph::new("").style(Style::default().bg(Theme::MANTLE))
                }
            }
        };
        frame.render_widget(status_line, chunks[1]);

        let help_text = match &self.mode {
            InputMode::Install => " Enter Install  Esc Cancel",
            InputMode::ConfirmUninstall => " y Confirm  any other key Cancel",
            InputMode::Normal => " i Install  x Remove  r Refresh  (toggle per-agent in Agents > Skills)",
        };
        frame.render_widget(
            Paragraph::new(help_text).style(Style::default().fg(Theme::OVERLAY0).bg(Theme::MANTLE)),
            chunks[2],
        );
    }
}
