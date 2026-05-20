use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::{App, COMMANDS};
use crate::tools::{ACCENT, BG, BG2, BG3, BORDER, MUTED};

#[derive(Debug, Default)]
pub struct HomeState {
    pub selected: usize,
}

pub async fn handle(app: &mut App, code: KeyCode) {
    const COLS: usize = 2;
    let len = crate::app::HOME_TOOLS.len();
    match code {
        KeyCode::Char('j') | KeyCode::Down => {
            let row = app.home.selected / COLS;
            let col = app.home.selected % COLS;
            let new_idx = (row + 1) * COLS + col;
            if new_idx < len {
                app.home.selected = new_idx;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let row = app.home.selected / COLS;
            let col = app.home.selected % COLS;
            if row > 0 {
                app.home.selected = (row - 1) * COLS + col;
            }
        }
        KeyCode::Char('l') | KeyCode::Right => {
            let row = app.home.selected / COLS;
            let new_idx = row * COLS + 1;
            if new_idx < len {
                app.home.selected = new_idx;
            }
        }
        KeyCode::Char('h') | KeyCode::Left => {
            let row = app.home.selected / COLS;
            app.home.selected = row * COLS;
        }
        KeyCode::Enter => {
            let cmd = crate::app::HOME_TOOLS
                .get(app.home.selected)
                .map(|(_, cmd)| *cmd)
                .unwrap_or("");
            if !cmd.is_empty() {
                crate::app::execute_command(app, cmd).await;
            }
        }
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            app.screen = crate::app::Screen::Quitting;
        }
        _ => {}
    }
}


pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    f.render_widget(Block::default().style(Style::default().bg(BG)), area);

    // Use localpart (not full MXID) as display name fallback.
    let display_name = app
        .accounts_tool
        .display_name
        .as_deref()
        .unwrap_or_else(|| {
            app.current_user_id
                .as_deref()
                .and_then(|uid| uid.trim_start_matches('@').split(':').next())
                .unwrap_or("—")
        });

    let account_str = app.current_user_id.clone().unwrap_or_else(|| "not signed in".to_owned());

    let room_count = app.rooms_tool.rooms.len();
    let total_unread: u64 = app.rooms_tool.rooms.iter().map(|r| r.unread).sum();
    let total_mentions: u64 = app.rooms_tool.rooms.iter().map(|r| r.mentions).sum();

    let device_count = app.accounts_tool.devices.len();

    let show_commands: Vec<(&str, &str)> = COMMANDS
        .iter()
        .filter(|(cmd, _)| !matches!(*cmd, "help" | "login" | "home" | "quit"))
        .map(|(cmd, desc)| (*cmd, *desc))
        .collect();
    let cmd_rows = show_commands.chunks(2).count() as u16;

    // Layout — top/left/right padding is applied by draw_main; home just defines content rows.
    let chunks = Layout::vertical([
        Constraint::Length(1), // [0] "WELCOME BACK" label
        Constraint::Length(1), // [1] blank
        Constraint::Length(1), // [2] display name
        Constraint::Length(1), // [3] blank
        Constraint::Length(1), // [4] subtitle ("signed in as …")
        Constraint::Length(1), // [5] gap
        Constraint::Length(5), // [6] stats grid
        Constraint::Length(2), // [7] gap before commands
        Constraint::Length(1), // [8] "COMMANDS" header
        Constraint::Length(1), // [9] blank line
        Constraint::Length(cmd_rows), // [10] command rows
        Constraint::Min(0),    // [11] trailing space
    ])
    .split(area);

    let pad = area.x;
    let w = area.width;

    f.render_widget(
        Paragraph::new("WELCOME BACK").style(Style::default().fg(MUTED)),
        Rect::new(pad, chunks[0].y, w, 1),
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▌ ", Style::default().fg(ACCENT)),
            Span::styled(
                display_name.to_owned(),
                Style::default().fg(Color::Rgb(237, 239, 242)).add_modifier(Modifier::BOLD),
            ),
        ])),
        Rect::new(pad, chunks[2].y, w, 1),
    );

    let subtitle_spans = if app.matrix.is_some() {
        vec![
            Span::styled("signed in as ", Style::default().fg(Color::Rgb(79, 87, 94))),
            Span::styled(account_str, Style::default().fg(MUTED)),
            Span::styled("  ·  session restored", Style::default().fg(Color::Rgb(79, 87, 94))),
        ]
    } else {
        vec![Span::styled("not signed in", Style::default().fg(Color::Rgb(79, 87, 94)))]
    };
    f.render_widget(
        Paragraph::new(Line::from(subtitle_spans)),
        Rect::new(pad, chunks[4].y, w, 1),
    );

    // Stats grid: 1-char gaps between boxes, centered content.
    let stats_area = Rect::new(pad, chunks[6].y, w, chunks[6].height);
    let stat_cols = Layout::horizontal([
        Constraint::Ratio(1, 4),
        Constraint::Length(1),
        Constraint::Ratio(1, 4),
        Constraint::Length(1),
        Constraint::Ratio(1, 4),
        Constraint::Length(1),
        Constraint::Ratio(1, 4),
    ])
    .split(stats_area);

    let render_stat = |f: &mut Frame, area: Rect, label: &str, value: &str| {
        let inner = Rect::new(area.x + 1, area.y + 1, area.width.saturating_sub(2), area.height.saturating_sub(2));
        // label | gap (expands) | value — no top/bottom padding, gap sits between
        let inner_chunks = Layout::vertical([
            Constraint::Length(1), // label
            Constraint::Min(0),    // gap
            Constraint::Length(1), // value
        ])
        .split(inner);

        f.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER))
                .style(Style::default().bg(BG2)),
            area,
        );
        f.render_widget(
            Paragraph::new(label)
                .style(Style::default().fg(MUTED).bg(BG2))
                .alignment(Alignment::Center),
            inner_chunks[0],
        );
        f.render_widget(
            Paragraph::new(value.to_owned())
                .style(Style::default().fg(Color::Rgb(237, 239, 242)).bg(BG2).add_modifier(Modifier::BOLD))
                .alignment(Alignment::Center),
            inner_chunks[2],
        );
    };

    let device_value = if device_count > 0 { device_count.to_string() } else { "—".to_owned() };

    render_stat(f, stat_cols[0], "ROOMS JOINED", &room_count.to_string());
    render_stat(f, stat_cols[2], "UNREAD", &total_unread.to_string());
    render_stat(f, stat_cols[4], "MENTIONS", &total_mentions.to_string());
    render_stat(f, stat_cols[6], "DEVICES", &device_value);

    // Commands section.
    f.render_widget(
        Paragraph::new("COMMANDS").style(Style::default().fg(MUTED)),
        Rect::new(pad, chunks[8].y, w, 1),
    );

    let cmd_area = Rect::new(pad, chunks[10].y, w, chunks[10].height);
    for (row_idx, pair) in show_commands.chunks(2).enumerate() {
        let row_y = cmd_area.y + row_idx as u16;
        if row_y >= cmd_area.y + cmd_area.height {
            break;
        }
        let cols = Layout::horizontal([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
            .split(Rect::new(cmd_area.x, row_y, cmd_area.width, 1));

        for (col_idx, (cmd, desc)) in pair.iter().enumerate() {
            let line = Line::from(vec![
                Span::styled(format!(":{cmd}"), Style::default().fg(ACCENT)),
                Span::styled(format!("  {desc}"), Style::default().fg(Color::Rgb(115, 125, 133))),
            ]);
            let cell_area = Rect::new(cols[col_idx].x, row_y, cols[col_idx].width, 1);

            let selected_cmd = crate::app::HOME_TOOLS
                .get(app.home.selected)
                .map(|(_, c)| *c)
                .unwrap_or("");
            let is_selected = *cmd == selected_cmd;
            let style = if is_selected { Style::default().bg(BG3) } else { Style::default().bg(BG) };
            f.render_widget(Paragraph::new(line).style(style), cell_area);
        }
    }
}

pub fn hint_spans() -> Vec<Span<'static>> {
    vec![
        Span::styled("↑↓←→", Style::default().fg(ACCENT)),
        Span::raw(" navigate  "),
        Span::styled("Enter", Style::default().fg(ACCENT)),
        Span::raw(" open  "),
        Span::styled(":", Style::default().fg(ACCENT)),
        Span::raw(" cmd  "),
        Span::styled("?", Style::default().fg(ACCENT)),
        Span::raw(" help  "),
        Span::styled("q", Style::default().fg(ACCENT)),
        Span::raw(" quit"),
    ]
}
