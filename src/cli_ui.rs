//! Styled CLI output helpers — box-drawing, colored sections, progress indicators.
//! Uses ANSI escape codes matching Catppuccin Mocha palette.

// ── Catppuccin Mocha ANSI colors ──
pub const MAUVE: &str = "\x1b[38;2;203;166;247m";
pub const LAVENDER: &str = "\x1b[38;2;180;190;254m";
#[allow(dead_code)]
pub const BLUE: &str = "\x1b[38;2;137;180;250m";
pub const SAPPHIRE: &str = "\x1b[38;2;116;199;236m";
pub const TEAL: &str = "\x1b[38;2;148;226;213m";
pub const GREEN: &str = "\x1b[38;2;166;227;161m";
pub const YELLOW: &str = "\x1b[38;2;249;226;175m";
pub const RED: &str = "\x1b[38;2;243;139;168m";
pub const TEXT: &str = "\x1b[38;2;205;214;244m";
pub const SUBTEXT: &str = "\x1b[38;2;166;173;200m";
pub const OVERLAY: &str = "\x1b[38;2;127;132;156m";
pub const SURFACE: &str = "\x1b[38;2;69;71;90m";
pub const DIM: &str = "\x1b[2m";
pub const BOLD: &str = "\x1b[1m";
pub const RESET: &str = "\x1b[0m";

const BOX_WIDTH: usize = 56;

/// Print a section header with rounded box-drawing border.
/// ```text
/// ╭─── 🔧 Claude Code CLI ──────────────────────────────╮
/// ```
pub fn section_header(icon: &str, title: &str) {
    let title_part = format!(" {} {} ", icon, title);
    // Icon may be multi-byte (emoji = 2 display cols), title_part.len() counts bytes.
    // Approximate visible width: icon≈2 + spaces + title chars
    let visible_title_len = 2 + 2 + title.len(); // " icon title "
    let right_dashes = BOX_WIDTH.saturating_sub(3 + visible_title_len + 1); // ╭───title...╮
    println!(
        "  {}╭───{}{}{}{}{}{}{}╮{}",
        SURFACE, RESET, MAUVE, BOLD, title_part, RESET,
        SURFACE, "─".repeat(right_dashes),
        RESET
    );
}

/// Print a section footer.
pub fn section_footer() {
    println!(
        "  {}╰{}╯{}",
        SURFACE,
        "─".repeat(BOX_WIDTH),
        RESET,
    );
    println!();
}

/// Print a line inside a section box.
pub fn section_line(text: &str) {
    println!(
        "  {}│{}  {}{}",
        SURFACE, RESET, text, RESET,
    );
}

/// Print an empty line inside a section.
pub fn section_empty() {
    println!("  {}│{}", SURFACE, RESET);
}

/// Print a success checkmark line inside a section.
pub fn section_ok(text: &str) {
    println!(
        "  {}│{}  {}✓{} {}{}{}",
        SURFACE, RESET, GREEN, RESET, TEXT, text, RESET
    );
}

/// Print a warning line inside a section.
pub fn section_warn(text: &str) {
    println!(
        "  {}│{}  {}⚠{} {}{}{}",
        SURFACE, RESET, YELLOW, RESET, YELLOW, text, RESET
    );
}

/// Print an error line inside a section.
pub fn section_err(text: &str) {
    println!(
        "  {}│{}  {}✗{} {}{}{}",
        SURFACE, RESET, RED, RESET, RED, text, RESET
    );
}

/// Print an info/hint line inside a section (dimmed).
pub fn section_hint(text: &str) {
    println!(
        "  {}│{}  {}{}{}",
        SURFACE, RESET, OVERLAY, text, RESET
    );
}

/// Print a key=value line inside a section.
#[allow(dead_code)]
pub fn section_kv(key: &str, value: &str) {
    println!(
        "  {}│{}  {}{:<16}{} {}{}{}",
        SURFACE, RESET, SUBTEXT, key, RESET, TEXT, value, RESET
    );
}

/// Print a decorated divider inside a section.
pub fn section_divider() {
    println!(
        "  {}│  {}{}",
        SURFACE,
        "─".repeat(BOX_WIDTH - 3),
        RESET
    );
}

/// Print a step indicator with progress dots and label.
/// ```text
///   ● ● ○   Step 2/3 — Agent Setup
/// ```
pub fn step_indicator(current: usize, total: usize, label: &str) {
    let dots: String = (1..=total)
        .map(|i| {
            if i < current {
                format!("{}●{}", GREEN, RESET)
            } else if i == current {
                format!("{}◉{}", MAUVE, RESET)
            } else {
                format!("{}○{}", SURFACE, RESET)
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    println!(
        "  {} {}  {}Step {}/{}{}  {}─{} {}{}{}",
        dots, RESET, DIM, current, total, RESET, SURFACE, RESET, LAVENDER, label, RESET
    );
    println!();
}

/// Print the final summary card.
/// ```text
///   ╔══════════════════════════════════════════════════════╗
///   ║  ✨ CatClaw Ready                                   ║
///   ╟──────────────────────────────────────────────────────╢
///   ║  Config      ./catclaw.toml                         ║
///   ║  Agents      main                                   ║
///   ║  Channels    discord                                ║
///   ╚══════════════════════════════════════════════════════╝
/// ```
pub fn summary_box(lines: &[(&str, &str)]) {
    println!(
        "  {}╔{}╗{}",
        MAUVE, "═".repeat(BOX_WIDTH), RESET
    );
    println!(
        "  {}║{}  {}{}✨ CatClaw Ready{}",
        MAUVE, RESET, BOLD, GREEN, RESET
    );
    println!(
        "  {}╟{}╢{}",
        MAUVE, "─".repeat(BOX_WIDTH), RESET,
    );

    for (key, value) in lines {
        println!(
            "  {}║{}  {}{:<14}{} {}{}{}",
            MAUVE, RESET, SUBTEXT, key, RESET, TEXT, value, RESET
        );
    }

    println!(
        "  {}╚{}╝{}",
        MAUVE, "═".repeat(BOX_WIDTH), RESET,
    );
    println!();
}

/// Print styled text for post-init status messages.
pub fn status_msg(icon: &str, text: &str) {
    println!(
        "  {} {}{}{}",
        icon, TEXT, text, RESET
    );
}

/// Braille spinner frames
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Start a spinner line (prints the initial frame).
/// Subsequent calls to `spinner_update` or `spinner_finish` overwrite this line.
pub fn spinner_start(text: &str) {
    use std::io::Write;
    print!(
        "  {}{}{} {}{}{}",
        MAUVE, SPINNER_FRAMES[0], RESET, SUBTEXT, text, RESET
    );
    std::io::stdout().flush().ok();
}

/// Update the spinner line with new text (overwrites current line).
#[allow(dead_code)]
pub fn spinner_update(text: &str) {
    use std::io::Write;
    // Use a simple frame based on current time for animation
    let frame_idx = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() / 80) as usize % SPINNER_FRAMES.len();
    print!(
        "\r  {}{}{} {}{}{}  \x1b[K",
        MAUVE, SPINNER_FRAMES[frame_idx], RESET, SUBTEXT, text, RESET
    );
    std::io::stdout().flush().ok();
}

/// Finish the spinner line — replace with final icon + text, then newline.
pub fn spinner_finish(icon: &str, text: &str) {
    println!(
        "\r  {} {}{}{}  \x1b[K",
        icon, TEXT, text, RESET
    );
}

/// Interactive inline selector rendered inside a section box.
/// Uses crossterm raw mode for arrow-key navigation with ◉/○ indicators.
/// Returns the index of the selected item.
pub fn section_select(items: &[&str], default: usize) -> usize {
    use crossterm::{cursor, event, execute, terminal};
    use std::io::{self, Write};

    let mut selected = default.min(items.len().saturating_sub(1));
    let item_count = items.len();

    // Print initial items inside the box
    for (i, item) in items.iter().enumerate() {
        let (icon, color) = if i == selected {
            ("◉", MAUVE)
        } else {
            ("○", OVERLAY)
        };
        println!(
            "  {}│{}  {}{}  {}{}{}",
            SURFACE, RESET, color, icon, TEXT, item, RESET
        );
    }

    // Enter raw mode for key capture
    terminal::enable_raw_mode().ok();

    // Move cursor up to redraw items in-place
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    loop {
        if let Ok(evt) = event::read() {
            match evt {
                event::Event::Key(key) => match key.code {
                    event::KeyCode::Up | event::KeyCode::Char('k') => {
                        selected = selected.saturating_sub(1);
                    }
                    event::KeyCode::Down | event::KeyCode::Char('j') => {
                        if selected + 1 < item_count {
                            selected += 1;
                        }
                    }
                    event::KeyCode::Enter => break,
                    event::KeyCode::Esc => break,
                    _ => continue,
                },
                _ => continue,
            }

            // Move cursor up and redraw all items
            execute!(
                stdout,
                cursor::MoveUp(item_count as u16),
                cursor::MoveToColumn(0)
            )
            .ok();

            for (i, item) in items.iter().enumerate() {
                let (icon, color) = if i == selected {
                    ("◉", MAUVE)
                } else {
                    ("○", OVERLAY)
                };
                // Clear line and print
                execute!(stdout, terminal::Clear(terminal::ClearType::CurrentLine)).ok();
                write!(
                    stdout,
                    "  {}│{}  {}{}  {}{}{}\r\n",
                    SURFACE, RESET, color, icon, TEXT, item, RESET
                )
                .ok();
            }
            stdout.flush().ok();
        }
    }

    terminal::disable_raw_mode().ok();
    selected
}

/// Show a yes/no confirmation prompt. Returns true if user selects "Yes".
/// `default` — which option is pre-selected (true = Yes, false = No).
pub fn section_confirm(prompt: &str, default: bool) -> bool {
    println!("  {}│{}  {}{}", SURFACE, RESET, TEXT, prompt);
    let default_idx = if default { 0 } else { 1 };
    section_select(&["Yes", "No"], default_idx) == 0
}
