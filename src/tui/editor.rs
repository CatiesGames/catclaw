use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;
use tui_textarea::TextArea;

use super::theme::Theme;

/// Full-screen Markdown editor using tui-textarea
#[allow(dead_code)]
pub struct MdEditor<'a> {
    textarea: TextArea<'a>,
    title: String,
    file_path: String,
    modified: bool,
}

#[allow(dead_code)]
impl<'a> MdEditor<'a> {
    pub fn new(title: &str, file_path: &str, content: &str) -> Self {
        let lines: Vec<String> = content.lines().map(String::from).collect();
        let mut textarea = TextArea::new(if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        });

        textarea.set_cursor_line_style(Style::default().bg(Theme::SURFACE0));
        textarea.set_line_number_style(Style::default().fg(Theme::OVERLAY0));

        MdEditor {
            textarea,
            title: title.to_string(),
            file_path: file_path.to_string(),
            modified: false,
        }
    }

    pub fn handle_event(&mut self, event: &KeyEvent) -> EditorAction {
        match (event.modifiers, event.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
                return EditorAction::Save;
            }
            (KeyModifiers::CONTROL, KeyCode::Char('q')) => {
                return EditorAction::Quit;
            }
            _ => {
                let changed = self.textarea.input(*event);
                if changed {
                    self.modified = true;
                }
            }
        }
        EditorAction::None
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);

        let modified_indicator = if self.modified { " [modified]" } else { "" };

        self.textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Theme::SURFACE1))
                .title(format!(
                    " {} — {}{} ",
                    self.title, self.file_path, modified_indicator
                ))
                .title_style(Style::default().fg(Theme::MAUVE)),
        );

        frame.render_widget(&self.textarea, chunks[0]);

        let help = Paragraph::new(" ⌃S Save  ⌃Q Close")
            .style(Style::default().fg(Theme::OVERLAY0).bg(Theme::MANTLE));
        frame.render_widget(help, chunks[1]);
    }

    pub fn content(&self) -> String {
        self.textarea.lines().join("\n")
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }
}

#[allow(dead_code)]
pub enum EditorAction {
    None,
    Save,
    Quit,
}
