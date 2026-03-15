use ratatui::prelude::*;
use ratatui::widgets::*;

use super::theme::Theme;

/// A single chat message for display
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub sender: String,
    pub text: String,
    pub is_user: bool,
    pub timestamp: String,
    /// True while this message is still receiving streaming tokens
    pub streaming: bool,
}

/// 6-dot braille loading frames
const LOADING_FRAMES: &[&str] = &[
    "⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏",
];

/// Render chat messages with scroll support.
pub fn render_chat(
    frame: &mut Frame,
    area: Rect,
    messages: &[ChatMessage],
    loading: bool,
    scroll: &mut u16,
    tick: u64,
    elapsed_secs: u64,
) {
    if area.width < 4 || area.height < 2 {
        return;
    }

    let max_width = (area.width as usize).saturating_sub(6);
    let mut lines: Vec<Line> = Vec::new();

    for msg in messages {
        let (name_color, bar_color) = if msg.is_user {
            (Theme::SAPPHIRE, Theme::SAPPHIRE)
        } else if msg.sender == "error" {
            (Theme::RED, Theme::RED)
        } else {
            (Theme::MAUVE, Theme::MAUVE)
        };

        // Sender header
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                &msg.sender,
                Style::default().fg(name_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(&msg.timestamp, Style::default().fg(Theme::OVERLAY0)),
        ]));

        // Message body with left bar
        let wrapped = wrap_text(&msg.text, max_width.saturating_sub(4));
        let last_idx = wrapped.len().saturating_sub(1);
        for (i, text_line) in wrapped.into_iter().enumerate() {
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled("│ ", Style::default().fg(bar_color)),
                Span::styled(text_line, Style::default().fg(Theme::TEXT)),
            ];
            // Show blinking cursor at end of last line when streaming
            if msg.streaming && i == last_idx {
                spans.push(Span::styled("▌", Style::default().fg(Theme::MAUVE)));
            }
            lines.push(Line::from(spans));
        }

        lines.push(Line::from(""));
    }

    // Loading indicator — braille spinner with elapsed time
    if loading {
        let frame_idx = (tick / 3) as usize % LOADING_FRAMES.len();
        let spinner = LOADING_FRAMES[frame_idx];

        let time_str = if elapsed_secs > 0 {
            format!(" ({}s)", elapsed_secs)
        } else {
            String::new()
        };

        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("{} ", spinner),
                Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Thinking...",
                Style::default().fg(Theme::OVERLAY0).add_modifier(Modifier::ITALIC),
            ),
            Span::styled(
                time_str,
                Style::default().fg(Theme::SURFACE2),
            ),
        ]));
        lines.push(Line::from(""));
    }

    // Auto-scroll to bottom
    let total_lines = lines.len() as u16;
    let max_scroll = total_lines.saturating_sub(area.height);
    if *scroll > max_scroll {
        *scroll = max_scroll;
    }

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((*scroll, 0));

    frame.render_widget(paragraph, area);
}

/// Word-wrap text into lines of max_width.
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }

    let mut result = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            result.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in line.split_whitespace() {
            if current.is_empty() {
                if word.len() > max_width {
                    let mut rem = word;
                    while rem.len() > max_width {
                        result.push(rem[..max_width].to_string());
                        rem = &rem[max_width..];
                    }
                    current = rem.to_string();
                } else {
                    current = word.to_string();
                }
            } else if current.len() + 1 + word.len() > max_width {
                result.push(current);
                current = word.to_string();
            } else {
                current.push(' ');
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            result.push(current);
        }
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}
