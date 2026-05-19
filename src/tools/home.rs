use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::{App, COMMANDS};
use crate::tools::{ACCENT, BG, BG2, BG3, BORDER, DANGER, MUTED};

#[derive(Debug, Default)]
pub struct HomeState {
    pub selected: usize,
}

pub async fn handle(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('j') | KeyCode::Down => {
            if app.home.selected + 1 < crate::app::HOME_TOOLS.len() {
                app.home.selected += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.home.selected > 0 {
                app.home.selected -= 1;
            }
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

    // Derive display info from app state.
    let display_name = app
        .profile
        .display_name
        .as_deref()
        .or(app.current_user_id.as_deref())
        .unwrap_or("—");

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
        _ => "not signed in".to_owned(),
    };

    let room_count = app.rooms_tool.rooms.len();
    let total_unread: u64 = app.rooms_tool.rooms.iter().map(|r| r.unread).sum();
    let total_mentions: u64 = app.rooms_tool.rooms.iter().map(|r| r.mentions).sum();

    let server_count = {
        let servers: std::collections::HashSet<&str> = app
            .rooms_tool
            .rooms
            .iter()
            .filter_map(|r| r.id.split(':').nth(1))
            .collect();
        servers.len()
    };
    let rooms_subtitle = if server_count <= 1 {
        "joined rooms".to_owned()
    } else {
        format!("across {} servers", server_count)
    };

    let device_count = app.devices.devices.len();
    let current_devices = app.devices.devices.iter().filter(|d| d.is_current).count();
    let other_devices = device_count.saturating_sub(current_devices);
    let devices_subtitle = if device_count == 0 {
        "—".to_owned()
    } else if other_devices > 0 {
        format!("{} current · {} others", current_devices, other_devices)
    } else {
        format!("{} device(s)", device_count)
    };

    // Commands to display: skip "help", "login", "home", "quit".
    let show_commands: Vec<(&str, &str)> = COMMANDS
        .iter()
        .filter(|(cmd, _)| !matches!(*cmd, "help" | "login" | "home" | "quit"))
        .map(|(cmd, desc)| (*cmd, *desc))
        .collect();
    let cmd_rows = show_commands.chunks(2).count() as u16;

    // Layout.
    let sep_width = area.width.saturating_sub(4);
    let chunks = Layout::vertical([
        Constraint::Length(1), // padding
        Constraint::Length(1), // "WELCOME BACK" label
        Constraint::Length(1), // display name
        Constraint::Length(1), // subtitle
        Constraint::Length(1), // separator
        Constraint::Length(5), // stats grid
        Constraint::Length(1), // gap
        Constraint::Length(1), // "COMMANDS" header
        Constraint::Length(cmd_rows), // command rows
        Constraint::Min(0),    // padding
    ])
    .split(area);

    // Hero section.
    f.render_widget(
        Paragraph::new("WELCOME BACK")
            .style(Style::default().fg(MUTED))
            .alignment(Alignment::Left),
        Rect::new(area.x + 2, chunks[1].y, chunks[1].width.saturating_sub(4), 1),
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▌ ", Style::default().fg(ACCENT)),
            Span::styled(
                display_name.to_owned(),
                Style::default()
                    .fg(Color::Rgb(237, 239, 242))
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        Rect::new(area.x + 2, chunks[2].y, chunks[2].width.saturating_sub(4), 1),
    );

    let signed_in_spans = if app.matrix.is_some() {
        vec![
            Span::styled("signed in as ", Style::default().fg(Color::Rgb(79, 87, 94))),
            Span::styled(account_str, Style::default().fg(MUTED)),
            Span::styled("  ·  session restored", Style::default().fg(Color::Rgb(79, 87, 94))),
        ]
    } else {
        vec![Span::styled("not signed in", Style::default().fg(Color::Rgb(79, 87, 94)))]
    };
    f.render_widget(
        Paragraph::new(Line::from(signed_in_spans)),
        Rect::new(area.x + 2, chunks[3].y, chunks[3].width.saturating_sub(4), 1),
    );

    // Separator (dashed line).
    let sep_str = "─".repeat(sep_width as usize);
    f.render_widget(
        Paragraph::new(sep_str).style(Style::default().fg(BORDER)),
        Rect::new(area.x + 2, chunks[4].y, sep_width, 1),
    );

    // Stats grid: 4 equal columns.
    let stat_cols = Layout::horizontal([
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
    ])
    .split(chunks[5]);

    let render_stat = |f: &mut Frame, area: Rect, label: &str, value: &str, subtitle: &str, value_color: Color| {
        let inner = Rect::new(area.x + 1, area.y + 1, area.width.saturating_sub(2), area.height.saturating_sub(2));
        let inner_chunks = Layout::vertical([
            Constraint::Length(1), // label
            Constraint::Length(1), // value
            Constraint::Length(1), // subtitle
            Constraint::Min(0),    // padding
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
                .style(Style::default().fg(MUTED).bg(BG2)),
            inner_chunks[0],
        );
        f.render_widget(
            Paragraph::new(value.to_owned())
                .style(Style::default().fg(value_color).bg(BG2).add_modifier(Modifier::BOLD)),
            inner_chunks[1],
        );
        f.render_widget(
            Paragraph::new(subtitle.to_owned())
                .style(Style::default().fg(Color::Rgb(79, 87, 94)).bg(BG2)),
            inner_chunks[2],
        );
    };

    let unread_color = if total_unread > 0 { ACCENT } else { Color::Rgb(115, 125, 133) };
    let mentions_color = if total_mentions > 0 { DANGER } else { Color::Rgb(115, 125, 133) };
    let device_value = if device_count > 0 { device_count.to_string() } else { "—".to_owned() };
    let device_color = if device_count > 0 { Color::Rgb(237, 239, 242) } else { Color::Rgb(115, 125, 133) };

    render_stat(f, stat_cols[0], "ROOMS JOINED", &room_count.to_string(), &rooms_subtitle, Color::Rgb(237, 239, 242));
    render_stat(f, stat_cols[1], "UNREAD", &total_unread.to_string(), "messages", unread_color);
    render_stat(f, stat_cols[2], "MENTIONS", &total_mentions.to_string(), "highlights", mentions_color);
    render_stat(f, stat_cols[3], "DEVICES", &device_value, &devices_subtitle, device_color);

    // Commands section.
    f.render_widget(
        Paragraph::new("COMMANDS").style(Style::default().fg(MUTED)),
        Rect::new(area.x + 2, chunks[7].y, chunks[7].width.saturating_sub(4), 1),
    );

    let cmd_area = Rect::new(area.x + 2, chunks[8].y, chunks[8].width.saturating_sub(4), chunks[8].height);
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

            // Highlight selected command.
            let selected_cmd = crate::app::HOME_TOOLS
                .get(app.home.selected)
                .map(|(_, c)| *c)
                .unwrap_or("");
            let is_selected = *cmd == selected_cmd;
            let style = if is_selected {
                Style::default().bg(BG3)
            } else {
                Style::default().bg(BG)
            };
            f.render_widget(Paragraph::new(line).style(style), cell_area);
        }
    }
}

pub fn hint_spans() -> Vec<Span<'static>> {
    vec![
        Span::styled(":", Style::default().fg(ACCENT)),
        Span::raw(" cmd  "),
        Span::styled("?", Style::default().fg(ACCENT)),
        Span::raw(" help  "),
        Span::styled("q", Style::default().fg(ACCENT)),
        Span::raw(" quit"),
    ]
}
