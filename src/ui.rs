use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::{self, App, CommandBarState, COMMANDS, Screen};
use crate::tools::{self, ACCENT, ACCENT_DIM, BG, BG2, BG3, BORDER, DANGER, FG, MUTED};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    Rect::new(
        (area.width.saturating_sub(width)) / 2 + area.x,
        (area.height.saturating_sub(height)) / 2 + area.y,
        width.min(area.width),
        height.min(area.height),
    )
}

/// Minimal filter input popup, floating at the bottom of `area`.
/// Column info and match count live in the status bar — this shows only the input.
/// Only call when `filter.active == true`.
pub fn draw_filter_popup(
    f: &mut Frame,
    filter: &tools::FilterState,
    area: Rect,
) {
    let input = &filter.input;
    let input_len = input.chars().count() as u16;
    // "  /  "(5) + input + "█"(1) + "  "(2)
    let width = (5u16 + input_len + 1 + 2).max(16).min(area.width);
    let height = 3u16;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + area.height.saturating_sub(4);
    let popup = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  /  ", Style::default().fg(ACCENT_DIM)),
            Span::styled(input.to_owned(), Style::default().fg(FG)),
            Span::styled("█", Style::default().fg(ACCENT_DIM)),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ACCENT))
                .style(Style::default().bg(BG2)),
        ),
        popup,
    );
}

// ---------------------------------------------------------------------------
// Top-level draw
// ---------------------------------------------------------------------------

pub fn draw(f: &mut Frame, app: &App) {
    match app.screen {
        Screen::Login => draw_login(f, app),
        Screen::Main => draw_main(f, app),
        Screen::Quitting => {}
    }
}

// ---------------------------------------------------------------------------
// Main layout: header + content + status bar
// ---------------------------------------------------------------------------

fn draw_main(f: &mut Frame, app: &App) {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(BG)), area);

    let footer_height: u16 = if app.command_bar.is_some() { 3 } else { 1 };
    let chunks = Layout::vertical([
        Constraint::Length(3), // header: 1 padding + 1 content + 1 padding
        Constraint::Min(1),
        Constraint::Length(footer_height),
    ])
    .split(area);

    draw_header(f, app, chunks[0]);

    // Uniform padding around all tool views: 1 row top/bottom, 2 chars left/right.
    let c = chunks[1];
    let padded = Rect::new(
        c.x + 2,
        c.y + 1,
        c.width.saturating_sub(4),
        c.height.saturating_sub(2),
    );

    use crate::app::ActiveTool::*;
    match app.active_tool {
        Home => tools::home::draw(f, app, padded),
        Rooms => tools::rooms::draw(f, app, padded),
        Accounts => tools::accounts::draw(f, app, padded),
    }

    draw_footer(f, app, chunks[2]);
    draw_toast(f, app, chunks[2].y);

    if app.show_help {
        tools::help::draw_overlay(f, app);
    }
}

fn draw_toast(f: &mut Frame, app: &App, footer_y: u16) {
    let Some((msg, color, at)) = &app.toast else { return; };
    if at.elapsed().as_secs() >= 3 { return; }

    let msg_w = msg.chars().count() as u16;
    let width = (msg_w + 6).min(f.area().width);
    let area = f.area();
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = footer_y.saturating_sub(5);
    let popup = Rect::new(x, y, width, 3);

    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {msg}  "),
            Style::default().fg(*color),
        )))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(*color))
                .style(Style::default().bg(BG2)),
        ),
        popup,
    );
}

// ---------------------------------------------------------------------------
// Header: ▌ matrix-account-tools  ·  :screen        ● sync  @user:hs
// ---------------------------------------------------------------------------

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    // Fill entire 3-row header with background.
    f.render_widget(Block::default().style(Style::default().bg(BG2)), area);

    // Content only on the middle row.
    let row = Rect::new(area.x, area.y + 1, area.width, 1);

    use crate::app::ActiveTool::*;
    let screen_name = match app.active_tool {
        Home => "home",
        Rooms => ":rooms",
        Accounts => ":accounts",
    };

    let account_str = match (&app.current_user_id, &app.matrix) {
        (Some(uid), Some(client)) => {
            let hs = client.homeserver_str();
            let hs_host = hs
                .trim_end_matches('/')
                .trim_start_matches("https://")
                .trim_start_matches("http://");
            let local = uid.trim_start_matches('@').split(':').next().unwrap_or(uid);
            format!("@{local}:{hs_host}")
        }
        (Some(uid), None) => uid.clone(),
        _ => String::new(),
    };

    // Right side: show "● Xs ago" once synced, "● syncing" until first sync, "● idle" if not connected.
    let (sync_label, sync_color) = match &app.matrix {
        Some(client) => match client.last_sync_at() {
            Some(t) => {
                let secs = t.elapsed().as_secs();
                let ago = match secs {
                    0..=59 => format!("{}s ago", secs),
                    60..=3599 => format!("{}m ago", secs / 60),
                    _ => format!("{}h ago", secs / 3600),
                };
                (format!(" ● {ago} "), ACCENT)
            }
            None => (" ● syncing ".to_owned(), MUTED),
        },
        None => (" ● idle ".to_owned(), MUTED),
    };

    let right_content = format!("{sync_label} {account_str} ");
    let right_len = right_content.chars().count() as u16;
    let cols = Layout::horizontal([Constraint::Min(1), Constraint::Length(right_len)])
        .split(row);

    let left_line = Line::from(vec![
        Span::styled("▌ ", Style::default().fg(ACCENT).bg(BG2)),
        Span::styled(
            "matrix-account-tools",
            Style::default().fg(Color::Rgb(237, 239, 242)).bg(BG2).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", Style::default().fg(MUTED).bg(BG2)),
        Span::styled(screen_name, Style::default().fg(ACCENT).bg(BG2)),
    ]);
    f.render_widget(
        Paragraph::new(left_line).style(Style::default().bg(BG2)),
        cols[0],
    );

    let right_line = Line::from(vec![
        Span::styled(sync_label, Style::default().fg(sync_color).bg(BG2)),
        Span::styled(format!(" {account_str} "), Style::default().fg(MUTED).bg(BG2)),
    ]);
    f.render_widget(
        Paragraph::new(right_line).alignment(Alignment::Right).style(Style::default().bg(BG2)),
        cols[1],
    );
}

// ---------------------------------------------------------------------------
// Footer: status bar or command palette
// ---------------------------------------------------------------------------

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    if let Some(bar) = &app.command_bar {
        draw_command_bar(f, bar, area);
    } else {
        draw_status_bar(f, app, area);
    }
}

fn draw_command_bar(f: &mut Frame, bar: &CommandBarState, area: Rect) {
    let completions = bar.completions();
    let comp_spans: Vec<Span> = COMMANDS
        .iter()
        .flat_map(|(cmd, _)| {
            let matched = completions.contains(cmd);
            let style = if matched {
                Style::default()
                    .fg(Color::Rgb(14, 20, 22))
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(MUTED)
            };
            vec![
                Span::styled(format!(" {cmd} "), style),
                Span::styled("  ", Style::default()),
            ]
        })
        .collect();

    f.render_widget(
        Paragraph::new(Line::from(comp_spans))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(BORDER)),
            )
            .style(Style::default().bg(BG2)),
        area,
    );

    let bottom = Rect::new(area.x, area.y + area.height.saturating_sub(1), area.width, 1);
    let input_line = Line::from(vec![
        Span::styled(":", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled(bar.input.clone(), Style::default().fg(Color::Rgb(237, 239, 242))),
        Span::styled("█", Style::default().fg(ACCENT)),
    ]);
    f.render_widget(
        Paragraph::new(input_line).style(Style::default().bg(BG2)),
        bottom,
    );
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    use crate::app::ActiveTool::*;

    // Fill background.
    f.render_widget(Block::default().style(Style::default().bg(BG3)), area);

    let mode = current_mode(app);
    let mode_text = format!(" {mode} ");
    let mode_width = mode_text.len() as u16;

    let screen_name = match app.active_tool {
        Home => "home",
        Rooms => ":rooms",
        Accounts => ":accounts",
    };
    let screen_text = format!("  {screen_name}  ");
    let screen_width = screen_text.len() as u16;

    let hints = match app.active_tool {
        Home => tools::home::hint_spans(),
        Rooms => tools::rooms::hint_spans(app),
        Accounts => tools::accounts::hint_spans(app),
    };

    let cols = Layout::horizontal([
        Constraint::Length(mode_width),
        Constraint::Length(screen_width),
        Constraint::Min(1),
    ])
    .split(area);

    // Mode badge: accent bg, dark text.
    let mode_color = match mode {
        "COMMAND" => Color::Rgb(77, 216, 168),
        "INSERT" => Color::Rgb(224, 160, 62),
        "LEAVE" => DANGER,
        "FILTER" => Color::Rgb(77, 160, 255),
        _ => ACCENT,
    };
    f.render_widget(
        Paragraph::new(mode_text)
            .style(Style::default().fg(Color::Rgb(14, 20, 22)).bg(mode_color).add_modifier(Modifier::BOLD)),
        cols[0],
    );

    // Screen name.
    f.render_widget(
        Paragraph::new(screen_text).style(Style::default().fg(MUTED).bg(BG3)),
        cols[1],
    );

    // Tool hints.
    f.render_widget(
        Paragraph::new(Line::from(hints)).style(Style::default().bg(BG3)),
        cols[2],
    );
}

fn current_mode(app: &App) -> &'static str {
    use crate::app::ActiveTool;
    if app.command_bar.is_some() {
        return "COMMAND";
    }
    if app::is_text_input_active(app) {
        return "INSERT";
    }
    match app.active_tool {
        ActiveTool::Rooms if app.rooms_tool.leave_select => "LEAVE",
        ActiveTool::Rooms if app.rooms_tool.filter.active => "FILTER",
        ActiveTool::Rooms
            if app.rooms_tool.detail_members_focused
                && app.rooms_tool.members.as_ref().map_or(false, |m| m.filter.active) =>
        {
            "FILTER"
        }
        ActiveTool::Accounts if app.accounts_tool.filter.active => "FILTER",
        ActiveTool::Accounts if app.accounts_tool.devices_filter.active => "FILTER",
        ActiveTool::Accounts if app.accounts_tool.ignored_filter.active => "FILTER",
        _ => "NORMAL",
    }
}

// ---------------------------------------------------------------------------
// Login screen
// ---------------------------------------------------------------------------

fn draw_login(f: &mut Frame, app: &App) {
    use crate::app::LoginField;

    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(BG)), area);

    let box_w = 62u16.min(area.width.saturating_sub(4));
    let box_h = 14u16.min(area.height.saturating_sub(2));
    let outer = Rect::new(
        (area.width.saturating_sub(box_w)) / 2,
        (area.height.saturating_sub(box_h)) / 2,
        box_w,
        box_h,
    );

    f.render_widget(
        Block::default()
            .title(Span::styled(
                if app.login.can_go_back { " add account " } else { " matrix-account-tools " },
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ))
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(BG2)),
        outer,
    );

    let inner = Rect::new(
        outer.x + 1,
        outer.y + 1,
        outer.width.saturating_sub(2),
        outer.height.saturating_sub(2),
    );

    // Layout: 1 padding + 3 fields (each 1 row + 1 separator) - last sep + 1 padding + 1 status + 1 hints
    let chunks = Layout::vertical([
        Constraint::Length(1), // top padding
        Constraint::Length(1), // homeserver field
        Constraint::Length(1), // separator
        Constraint::Length(1), // username field
        Constraint::Length(1), // separator
        Constraint::Length(1), // password field
        Constraint::Length(1), // bottom padding
        Constraint::Length(1), // status (loading/error)
        Constraint::Min(0),    // hints
    ])
    .split(inner);

    let sep_line = "─".repeat(inner.width as usize);
    let sep_style = Style::default().fg(BORDER).bg(BG2);

    let render_field = |f: &mut Frame, label: &str, value: &str, focused: bool, mask: bool, area: Rect| {
        let display: String = if mask { "•".repeat(value.chars().count()) } else { value.to_owned() };
        let cursor = if focused { "█" } else { "" };
        let label_color = if focused { ACCENT } else { MUTED };
        let row_bg = if focused { BG3 } else { BG2 };
        let bar = if focused {
            Span::styled("▌", Style::default().fg(ACCENT).bg(BG3))
        } else {
            Span::styled(" ", Style::default().bg(BG2))
        };
        let line = Line::from(vec![
            bar,
            Span::styled(
                format!(" {label:<12}"),
                Style::default().fg(label_color).bg(row_bg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(display, Style::default().fg(Color::Rgb(237, 239, 242)).bg(row_bg)),
            Span::styled(cursor, Style::default().fg(ACCENT).bg(row_bg)),
        ]);
        f.render_widget(Paragraph::new(line).style(Style::default().bg(row_bg)), area);
    };

    render_field(f, "HOMESERVER", &app.login.homeserver, app.login.focused == LoginField::Homeserver, false, chunks[1]);
    f.render_widget(Paragraph::new(sep_line.clone()).style(sep_style), chunks[2]);
    render_field(f, "USERNAME", &app.login.username, app.login.focused == LoginField::Username, false, chunks[3]);
    f.render_widget(Paragraph::new(sep_line).style(sep_style), chunks[4]);
    render_field(f, "PASSWORD", &app.login.password, app.login.focused == LoginField::Password, true, chunks[5]);

    if app.login.loading {
        f.render_widget(
            Paragraph::new("Logging in…")
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
                .alignment(Alignment::Center),
            chunks[7],
        );
    } else if let Some(err) = &app.login.error {
        f.render_widget(
            Paragraph::new(err.as_str())
                .style(Style::default().fg(DANGER))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            chunks[7],
        );
    }

    let hint = if app.login.can_go_back {
        "Tab next field  ·  Enter sign in  ·  Esc cancel"
    } else {
        "Tab next field  ·  Enter sign in  ·  Ctrl+C quit"
    };
    f.render_widget(
        Paragraph::new(hint)
            .style(Style::default().fg(MUTED))
            .alignment(Alignment::Center),
        chunks[8],
    );
}
