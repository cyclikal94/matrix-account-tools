use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, List, ListItem, ListState, Padding, Paragraph},
};

use super::{ACCENT, ACCENT_DIM, BG3, BORDER, DANGER, MUTED, FilterState, Filterable};

// ---------------------------------------------------------------------------
// Command table types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum CmdColor {
    Accent,
    Danger,
    Success,
    Muted,
}

impl CmdColor {
    pub const fn to_color(self) -> Color {
        match self {
            Self::Accent  => super::ACCENT,
            Self::Danger  => super::DANGER,
            Self::Success => super::SUCCESS,
            Self::Muted   => super::MUTED,
        }
    }
}

/// One entry in a tool's command table.
///
/// Declare `pub const CMDS: &[Cmd] = &[...]` in each tool module.
/// The table drives both the status-bar hints and the help overlay automatically.
#[derive(Debug, Clone, Copy)]
pub struct Cmd {
    pub key: &'static str,
    pub desc: &'static str,
    pub color: CmdColor,
}

impl Cmd {
    pub const fn new(key: &'static str, desc: &'static str) -> Self {
        Self { key, desc, color: CmdColor::Accent }
    }
    pub const fn danger(key: &'static str, desc: &'static str) -> Self {
        Self { key, desc, color: CmdColor::Danger }
    }
    pub const fn success(key: &'static str, desc: &'static str) -> Self {
        Self { key, desc, color: CmdColor::Success }
    }
    #[allow(dead_code)]
    pub const fn muted(key: &'static str, desc: &'static str) -> Self {
        Self { key, desc, color: CmdColor::Muted }
    }
}

// ---------------------------------------------------------------------------
// Hint spans from command table
// ---------------------------------------------------------------------------

/// Build the status-bar hint `Vec<Span<'static>>` from a CMDS slice.
///
/// Format: `key` (colored) then `" desc  "` (raw, two trailing spaces as separator).
pub fn hint_spans_from_cmds(cmds: &[Cmd]) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(cmds.len() * 2);
    for cmd in cmds {
        spans.push(Span::styled(cmd.key, Style::default().fg(cmd.color.to_color())));
        spans.push(Span::raw(format!(" {}  ", cmd.desc)));
    }
    spans
}

// ---------------------------------------------------------------------------
// Shared filter key handler
// ---------------------------------------------------------------------------

/// Handle all keypresses while a filter popup is active.
///
/// Covers Esc/Enter/Backspace/digits (column switch)/chars/j-k navigation.
/// Always returns `true` — all keys are consumed while the filter is active.
/// The caller checks `filter.active` first and should `return` after calling this.
pub fn handle_filter_keys(
    filter: &mut FilterState,
    selected: &mut usize,
    filtered_len: usize,
    n_cols: usize,
    code: KeyCode,
) -> bool {
    match code {
        KeyCode::Esc => filter.clear(),
        KeyCode::Enter => filter.active = false,
        KeyCode::Backspace => {
            filter.input.pop();
            *selected = 0;
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            let n = c.to_digit(10).unwrap() as usize;
            if n < n_cols {
                filter.column = if n == 0 { None } else { Some(n) };
                *selected = 0;
            }
        }
        KeyCode::Char(c) if !c.is_control() => {
            filter.input.push(c);
            *selected = 0;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if *selected + 1 < filtered_len {
                *selected += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if *selected > 0 {
                *selected -= 1;
            }
        }
        _ => {}
    }
    true
}

// ---------------------------------------------------------------------------
// Navigation helpers
// ---------------------------------------------------------------------------

/// Advance selection by 1, clamped to `len - 1`.
pub fn nav_down(selected: &mut usize, len: usize) {
    if *selected + 1 < len {
        *selected += 1;
    }
}

/// Move selection back by 1, clamped to 0.
pub fn nav_up(selected: &mut usize) {
    if *selected > 0 {
        *selected -= 1;
    }
}

// ---------------------------------------------------------------------------
// Panel focus colors
// ---------------------------------------------------------------------------

/// Border color for a panel: ACCENT if focused, BORDER if not.
#[allow(dead_code)]
pub fn panel_border_color(focused: bool) -> Color {
    if focused { ACCENT } else { BORDER }
}

/// Title color for a panel: ACCENT if focused, MUTED if not.
#[allow(dead_code)]
pub fn panel_title_color(focused: bool) -> Color {
    if focused { ACCENT } else { MUTED }
}

// ---------------------------------------------------------------------------
// Generic list panel scaffold
// ---------------------------------------------------------------------------

/// Draw a full filterable list panel.
///
/// Renders in order: loading spinner → empty-state message → filtered list with
/// title/border/padding → optional error strip at bottom → optional filter popup.
///
/// The caller is responsible for calling `draw_filter_popup` separately
/// (it needs the filter popup to appear on top of the list).
///
/// # Title format
/// `" Base Title (N) "` normally, `" Base Title (match/total) "` when filtering.
#[allow(dead_code)]
pub fn draw_list_block<T, F>(
    f: &mut Frame,
    base_title: &str,
    items: &[T],
    selected: usize,
    filter: &FilterState,
    loading: bool,
    focused: bool,
    error: &Option<String>,
    area: Rect,
    loading_msg: &'static str,
    empty_msg: &'static str,
    row_fn: F,
) where
    T: Filterable,
    F: Fn(&T) -> ListItem<'static>,
{
    if loading {
        f.render_widget(
            Paragraph::new(loading_msg)
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    if items.is_empty() {
        f.render_widget(
            Paragraph::new(empty_msg)
                .style(Style::default().fg(MUTED))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let filtered: Vec<&T> = items.iter().filter(|i| filter.matches_item(*i)).collect();
    let total = items.len();
    let match_count = filtered.len();

    let list_items: Vec<ListItem> = filtered.iter().map(|i| row_fn(i)).collect();

    let title = if !filter.input.is_empty() {
        format!(" {} ({}/{}) ", base_title, match_count, total)
    } else {
        format!(" {} ({}) ", base_title, total)
    };

    let border_color = panel_border_color(focused);
    let title_color = panel_title_color(focused);

    let list = List::new(list_items)
        .block(
            Block::default()
                .title(Span::styled(title, Style::default().fg(title_color)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .padding(Padding::new(1, 1, 1, 1)),
        )
        .highlight_style(
            Style::default()
                .bg(BG3)
                .fg(ACCENT_DIM)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▌ ");

    let mut state = ListState::default();
    state.select(Some(selected));

    match error {
        Some(err) => {
            let chunks = Layout::vertical([
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);
            f.render_stateful_widget(list, chunks[0], &mut state);
            f.render_widget(
                Paragraph::new(err.clone())
                    .style(Style::default().fg(DANGER))
                    .alignment(Alignment::Center),
                chunks[1],
            );
        }
        None => {
            f.render_stateful_widget(list, area, &mut state);
        }
    }
}

// ---------------------------------------------------------------------------
// Clipboard
// ---------------------------------------------------------------------------

/// Write `text` to the terminal clipboard via OSC 52 escape sequence.
pub fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    let b64 = encode_base64(text.as_bytes());
    let seq = format!("\x1b]52;c;{b64}\x07");
    let _ = std::io::stdout().write_all(seq.as_bytes());
    let _ = std::io::stdout().flush();
}

fn encode_base64(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(CHARS[(b0 >> 2) as usize] as char);
        out.push(CHARS[((b0 & 3) << 4 | b1 >> 4) as usize] as char);
        out.push(if chunk.len() > 1 { CHARS[((b1 & 0xf) << 2 | b2 >> 6) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { CHARS[(b2 & 0x3f) as usize] as char } else { '=' });
    }
    out
}
