use ratatui::prelude::*;
use ratatui::widgets::*;
use unicode_width::UnicodeWidthStr;

use super::theme::Theme;

/// Large ASCII art logo
const LOGO: &[&str] = &[
    r"                                                  ",
    r"     ██████╗  █████╗ ████████╗ ██████╗██╗      █████╗ ██╗    ██╗ ",
    r"    ██╔════╝ ██╔══██╗╚══██╔══╝██╔════╝██║     ██╔══██╗██║    ██║ ",
    r"    ██║      ███████║   ██║   ██║     ██║     ███████║██║ █╗ ██║ ",
    r"    ██║      ██╔══██║   ██║   ██║     ██║     ██╔══██║██║███╗██║ ",
    r"    ╚██████╗ ██║  ██║   ██║   ╚██████╗███████╗██║  ██║╚███╔███╔╝ ",
    r"     ╚═════╝ ╚═╝  ╚═╝   ╚═╝    ╚═════╝╚══════╝╚═╝  ╚═╝ ╚══╝╚══╝  ",
    r"                                                  ",
];

/// Gradient colors for the logo (top to bottom)
const LOGO_COLORS: &[Color] = &[
    Theme::MAUVE,    // purple
    Theme::MAUVE,
    Theme::LAVENDER, // light purple
    Theme::BLUE,     // blue
    Theme::SAPPHIRE, // cyan-blue
    Theme::TEAL,     // teal
    Theme::GREEN,    // green
    Theme::GREEN,
];

/// Small cat icon for tab bar
#[allow(dead_code)]
pub const CAT_ICON: &str = "🐱";

/// ANSI color codes matching the gradient for terminal printing
const ANSI_COLORS: &[&str] = &[
    "\x1b[38;2;203;166;247m", // MAUVE
    "\x1b[38;2;203;166;247m", // MAUVE
    "\x1b[38;2;180;190;254m", // LAVENDER
    "\x1b[38;2;137;180;250m", // BLUE
    "\x1b[38;2;116;199;236m", // SAPPHIRE
    "\x1b[38;2;148;226;213m", // TEAL
    "\x1b[38;2;166;227;161m", // GREEN
    "\x1b[38;2;166;227;161m", // GREEN
];
const ANSI_RESET: &str = "\x1b[0m";
const ANSI_MAUVE: &str = "\x1b[38;2;203;166;247m";
const ANSI_DIM: &str = "\x1b[38;2;127;132;156m";

/// Print the splash logo directly to terminal (no TUI, plain ANSI output).
/// Left-aligned with the logo's built-in leading spaces.
pub fn print_splash_to_terminal() {
    println!();
    for (i, line) in LOGO.iter().enumerate() {
        let color = ANSI_COLORS.get(i).unwrap_or(&ANSI_COLORS[0]);
        println!("{}{}{}", color, line, ANSI_RESET);
    }
    println!(
        "{}  v0.1.0{}  •  {}Personal AI Gateway powered by Claude Code{}",
        ANSI_DIM, ANSI_RESET, ANSI_DIM, ANSI_RESET
    );
    println!(
        "{}  Multi-agent • Multi-channel • Always-on{}",
        ANSI_MAUVE, ANSI_RESET
    );
    println!();
}

/// Calculate the display width of a Line (sum of span display widths)
fn line_display_width(line: &Line) -> u16 {
    line.spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()) as u16)
        .sum()
}

/// Render the splash screen (shown briefly on startup)
pub fn render_splash(frame: &mut Frame, area: Rect, tick: u16) {
    // Full dark background
    let bg = Block::default().style(Style::default().bg(Theme::CRUST));
    frame.render_widget(bg, area);

    // Calculate center position using display width
    let logo_height = LOGO.len() as u16;
    let info_height = 7u16; // version + tagline + hints
    let total_height = logo_height + info_height + 2;
    let logo_width = LOGO
        .iter()
        .map(|l| UnicodeWidthStr::width(*l) as u16)
        .max()
        .unwrap_or(0);

    let y_offset = area.height.saturating_sub(total_height) / 2;
    let x_offset = area.width.saturating_sub(logo_width) / 2;

    // Render logo lines with gradient
    for (i, line) in LOGO.iter().enumerate() {
        let y = y_offset + i as u16;
        if y >= area.height {
            break;
        }

        let color = LOGO_COLORS.get(i).copied().unwrap_or(Theme::MAUVE);

        // Fade-in effect: gradually reveal characters based on tick
        let total_chars = line.chars().count();
        let visible_chars = if tick < 8 {
            (total_chars as u16 * tick / 8) as usize
        } else {
            total_chars
        };

        let visible: String = line.chars().take(visible_chars).collect();

        let span = Span::styled(visible, Style::default().fg(color));
        let paragraph = Paragraph::new(Line::from(span));
        let line_area = Rect::new(x_offset, y, logo_width.min(area.width), 1);
        frame.render_widget(paragraph, line_area);
    }

    // Info section below logo
    let info_y = y_offset + logo_height + 1;
    if info_y + 7 < area.height {
        let version_line = Line::from(vec![
            Span::styled("v0.1.0", Style::default().fg(Theme::OVERLAY1)),
            Span::styled("  •  ", Style::default().fg(Theme::SURFACE2)),
            Span::styled(
                "Personal AI Gateway powered by Claude Code",
                Style::default().fg(Theme::OVERLAY1),
            ),
        ]);

        let tagline = Line::from(Span::styled(
            "Multi-agent • Multi-channel • Always-on",
            Style::default()
                .fg(Theme::MAUVE)
                .add_modifier(Modifier::ITALIC),
        ));

        // Decorative separator
        let sep_width = 48u16.min(area.width.saturating_sub(4));
        let sep_x = area.width.saturating_sub(sep_width) / 2;
        let sep: String = "─".repeat(sep_width as usize);
        let separator = Line::from(Span::styled(sep, Style::default().fg(Theme::SURFACE1)));

        let hints = Line::from(vec![
            Span::styled(
                "  Tab",
                Style::default()
                    .fg(Theme::MAUVE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Navigate   ", Style::default().fg(Theme::SUBTEXT0)),
            Span::styled(
                "q",
                Style::default()
                    .fg(Theme::MAUVE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Quit", Style::default().fg(Theme::SUBTEXT0)),
        ]);

        // Only show if tick is past initial animation
        if tick >= 6 {
            // Version line (centered by display width)
            let vw = line_display_width(&version_line);
            let vx = area.width.saturating_sub(vw) / 2;
            frame.render_widget(
                Paragraph::new(version_line),
                Rect::new(vx, info_y, area.width.saturating_sub(vx), 1),
            );

            // Tagline (centered)
            let tw = line_display_width(&tagline);
            let tx = area.width.saturating_sub(tw) / 2;
            frame.render_widget(
                Paragraph::new(tagline),
                Rect::new(tx, info_y + 1, area.width.saturating_sub(tx), 1),
            );

            // Separator
            frame.render_widget(
                Paragraph::new(separator),
                Rect::new(sep_x, info_y + 3, sep_width, 1),
            );

            // Hints (centered)
            let hw = line_display_width(&hints);
            let hx = area.width.saturating_sub(hw) / 2;
            frame.render_widget(
                Paragraph::new(hints),
                Rect::new(hx, info_y + 4, area.width.saturating_sub(hx), 1),
            );
        }
    }

    // Bottom credit line
    let bottom_y = area.height.saturating_sub(1);
    let credit = Line::from(vec![
        Span::styled(" catiesgames ", Style::default().fg(Theme::SURFACE2)),
        Span::styled("•", Style::default().fg(Theme::SURFACE1)),
        Span::styled(" catppuccin mocha ", Style::default().fg(Theme::SURFACE2)),
    ]);
    let credit_w = line_display_width(&credit);
    let credit_x = area.width.saturating_sub(credit_w) / 2;
    frame.render_widget(
        Paragraph::new(credit),
        Rect::new(credit_x, bottom_y, area.width.saturating_sub(credit_x), 1),
    );
}
