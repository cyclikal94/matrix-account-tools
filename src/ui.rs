use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::app::{App, CommandBarState, COMMANDS, Screen};
use crate::tools::{self, ACCENT, BG, FOCUSED, HEADER_BG, MUTED};

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
// Main layout: header + content + footer
// ---------------------------------------------------------------------------

fn draw_main(f: &mut Frame, app: &App) {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(BG)), area);

    let footer_height: u16 = if app.command_bar.is_some() { 3 } else { 2 };
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(footer_height),
    ])
    .split(area);

    draw_header(f, app, chunks[0]);

    use crate::app::ActiveTool::*;
    match app.active_tool {
        Home => tools::home::draw(f, app, chunks[1]),
        Rooms => tools::rooms::draw(f, app, chunks[1]),
        Accounts => tools::accounts::draw(f, app, chunks[1]),
        IgnoreList => tools::ignore_list::draw(f, app, chunks[1]),
        Profile => tools::profile::draw(f, app, chunks[1]),
        Devices => tools::devices::draw(f, app, chunks[1]),
    }

    draw_footer(f, app, chunks[2]);

    if app.show_help {
        tools::help::draw_overlay(f);
    }
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    use crate::app::ActiveTool::*;
    let tool_name = match app.active_tool {
        Home => tools::home::tool_name(),
        Rooms => tools::rooms::tool_name(),
        Accounts => tools::accounts::tool_name(),
        IgnoreList => tools::ignore_list::tool_name(),
        Profile => tools::profile::tool_name(),
        Devices => tools::devices::tool_name(),
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
        _ => "not logged in".to_owned(),
    };

    let spans = vec![
        Span::styled(
            " Matrix Account Tools ",
            Style::default()
                .fg(ACCENT)
                .bg(HEADER_BG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" │ ", Style::default().fg(MUTED).bg(HEADER_BG)),
        Span::styled(
            account_str,
            Style::default().fg(Color::White).bg(HEADER_BG),
        ),
        Span::styled(" │ ", Style::default().fg(MUTED).bg(HEADER_BG)),
        Span::styled(
            format!("{tool_name} "),
            Style::default()
                .fg(FOCUSED)
                .bg(HEADER_BG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("│ : command ", Style::default().fg(MUTED).bg(HEADER_BG)),
    ];

    f.render_widget(
        Paragraph::new(Line::from(spans))
            .style(Style::default().bg(HEADER_BG))
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(MUTED)),
            ),
        area,
    );
}

// ---------------------------------------------------------------------------
// Footer
// ---------------------------------------------------------------------------

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    if let Some(bar) = &app.command_bar {
        draw_command_bar(f, bar, area);
    } else {
        draw_hint_bar(f, app, area);
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
                    .fg(Color::Black)
                    .bg(FOCUSED)
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
                    .border_style(Style::default().fg(MUTED)),
            )
            .style(Style::default().bg(BG)),
        area,
    );

    let bottom = Rect::new(area.x, area.y + area.height.saturating_sub(1), area.width, 1);
    let input_line = Line::from(vec![
        Span::styled(":", Style::default().fg(FOCUSED).add_modifier(Modifier::BOLD)),
        Span::styled(bar.input.clone(), Style::default().fg(Color::White)),
        Span::styled("█", Style::default().fg(FOCUSED)),
    ]);
    f.render_widget(
        Paragraph::new(input_line).style(Style::default().bg(BG)),
        bottom,
    );
}

fn draw_hint_bar(f: &mut Frame, app: &App, area: Rect) {
    use crate::app::ActiveTool::*;
    let hints = match app.active_tool {
        Home => tools::home::hint_spans(),
        Rooms => tools::rooms::hint_spans(app),
        Accounts => tools::accounts::hint_spans(),
        IgnoreList => tools::ignore_list::hint_spans(app),
        Profile => tools::profile::hint_spans(app),
        Devices => tools::devices::hint_spans(app),
    };

    f.render_widget(
        Paragraph::new(Line::from(hints))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(MUTED)),
            ),
        area,
    );
}

// ---------------------------------------------------------------------------
// Login screen
// ---------------------------------------------------------------------------

fn draw_login(f: &mut Frame, app: &App) {
    use crate::app::LoginField;

    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(BG)), area);

    let box_w = 60u16.min(area.width.saturating_sub(4));
    let box_h = 22u16.min(area.height.saturating_sub(2));
    let outer = Rect::new(
        (area.width.saturating_sub(box_w)) / 2,
        (area.height.saturating_sub(box_h)) / 2,
        box_w,
        box_h,
    );

    f.render_widget(
        Block::default()
            .title(Span::styled(
                " Matrix Account Tools ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ))
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ACCENT)),
        outer,
    );

    let inner = Rect::new(
        outer.x + 1,
        outer.y + 1,
        outer.width.saturating_sub(2),
        outer.height.saturating_sub(2),
    );

    let chunks = Layout::vertical([
        Constraint::Length(1), // heading
        Constraint::Length(1), // gap
        Constraint::Length(3), // homeserver
        Constraint::Length(1), // gap
        Constraint::Length(3), // username
        Constraint::Length(1), // gap
        Constraint::Length(3), // password
        Constraint::Length(1), // gap
        Constraint::Length(2), // error / loading
        Constraint::Min(0),    // hints
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new(if app.login.can_go_back {
            "Add a Matrix account"
        } else {
            "Log in to your Matrix account"
        })
        .style(Style::default().fg(MUTED))
        .alignment(Alignment::Center),
        chunks[0],
    );

    let make_field = |label: &str, value: &str, focused: bool, mask: bool| {
        let display: String = if mask { "•".repeat(value.len()) } else { value.to_owned() };
        Paragraph::new(display)
            .style(if focused {
                Style::default().fg(FOCUSED)
            } else {
                Style::default().fg(Color::White)
            })
            .block(
                Block::default()
                    .title(Span::styled(
                        format!(" {label} "),
                        Style::default().fg(if focused { FOCUSED } else { ACCENT }),
                    ))
                    .borders(Borders::ALL)
                    .border_style(if focused {
                        Style::default().fg(FOCUSED)
                    } else {
                        Style::default().fg(ACCENT)
                    }),
            )
    };

    f.render_widget(
        make_field(
            "Homeserver URL",
            &app.login.homeserver,
            app.login.focused == LoginField::Homeserver,
            false,
        ),
        chunks[2],
    );
    f.render_widget(
        make_field(
            "Username",
            &app.login.username,
            app.login.focused == LoginField::Username,
            false,
        ),
        chunks[4],
    );
    f.render_widget(
        make_field(
            "Password",
            &app.login.password,
            app.login.focused == LoginField::Password,
            true,
        ),
        chunks[6],
    );

    if app.login.loading {
        f.render_widget(
            Paragraph::new("Logging in…")
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
                .alignment(Alignment::Center),
            chunks[8],
        );
    } else if let Some(err) = &app.login.error {
        f.render_widget(
            Paragraph::new(err.as_str())
                .style(Style::default().fg(tools::ERROR))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            chunks[8],
        );
    }

    let hint = if app.login.can_go_back {
        "Tab/↑↓ switch fields  •  Enter confirm  •  Esc cancel"
    } else {
        "Tab/↑↓ switch fields  •  Enter confirm  •  Ctrl+C quit"
    };
    f.render_widget(
        Paragraph::new(hint)
            .style(Style::default().fg(MUTED))
            .alignment(Alignment::Center),
        chunks[9],
    );

    let cursor_rect = match app.login.focused {
        LoginField::Homeserver => chunks[2],
        LoginField::Username => chunks[4],
        LoginField::Password => chunks[6],
    };
    let cursor_len = match app.login.focused {
        LoginField::Homeserver => app.login.homeserver.len(),
        LoginField::Username => app.login.username.len(),
        LoginField::Password => app.login.password.len(),
    };
    let cx = cursor_rect.x
        + 1
        + cursor_len.min((cursor_rect.width as usize).saturating_sub(3)) as u16;
    f.set_cursor_position((cx, cursor_rect.y + 1));
}
