use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use tokio::sync::oneshot;

use crate::app::{ActiveTool, App};
use crate::tools::{ACCENT, ACCENT_DIM, DANGER, MUTED, SUCCESS};
use crate::ui::centered_rect;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct IgnoreListState {
    pub users: Vec<String>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    /// Some(input) when the add-user prompt is active.
    pub add_prompt: Option<String>,
    pub confirm_unignore: bool,
    pub load_rx: Option<oneshot::Receiver<Result<Vec<String>, String>>>,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn handle(app: &mut App, code: KeyCode) {
    if app.ignore_list.loading {
        return;
    }

    // Add-user prompt.
    if let Some(ref mut input) = app.ignore_list.add_prompt.clone() {
        match code {
            KeyCode::Esc => app.ignore_list.add_prompt = None,
            KeyCode::Backspace => {
                let mut s = input.clone();
                s.pop();
                app.ignore_list.add_prompt = Some(s);
            }
            KeyCode::Char(c) if !c.is_control() => {
                let mut s = input.clone();
                s.push(c);
                app.ignore_list.add_prompt = Some(s);
            }
            KeyCode::Enter => {
                let user_id = input.trim().to_owned();
                if !user_id.starts_with('@') || !user_id.contains(':') {
                    app.ignore_list.error =
                        Some("Invalid user ID — must be @user:server".to_owned());
                    app.ignore_list.add_prompt = None;
                } else {
                    app.ignore_list.add_prompt = None;
                    if let Some(client) = &app.matrix {
                        match client.ignore_user(&user_id).await {
                            Ok(()) => app.ignore_list.error = None,
                            Err(e) => app.ignore_list.error = Some(format!("{e}")),
                        }
                    }
                    start_load(app);
                }
            }
            _ => {}
        }
        return;
    }

    // Confirm dialog.
    if app.ignore_list.confirm_unignore {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                do_unignore(app).await;
            }
            _ => {
                app.ignore_list.confirm_unignore = false;
            }
        }
        return;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
            app.active_tool = ActiveTool::Home;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if app.ignore_list.selected + 1 < app.ignore_list.users.len() {
                app.ignore_list.selected += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.ignore_list.selected > 0 {
                app.ignore_list.selected -= 1;
            }
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            app.ignore_list.add_prompt = Some(String::new());
            app.ignore_list.error = None;
        }
        KeyCode::Char('d') | KeyCode::Delete => {
            if !app.ignore_list.users.is_empty() {
                app.ignore_list.confirm_unignore = true;
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            start_load(app);
        }
        _ => {}
    }
}

async fn do_unignore(app: &mut App) {
    app.ignore_list.confirm_unignore = false;
    let user_id = match app.ignore_list.users.get(app.ignore_list.selected) {
        Some(u) => u.clone(),
        None => return,
    };
    if let Some(client) = &app.matrix {
        match client.unignore_user(&user_id).await {
            Ok(()) => app.ignore_list.error = None,
            Err(e) => app.ignore_list.error = Some(format!("{e}")),
        }
    }
    start_load(app);
}

pub fn start_load(app: &mut App) {
    let Some(client) = app.matrix.clone() else { return; };
    app.ignore_list.loading = true;
    app.ignore_list.error = None;
    let (tx, rx) = oneshot::channel();
    app.ignore_list.load_rx = Some(rx);
    tokio::spawn(async move {
        let result = client.get_ignored_users().await.map_err(|e| e.to_string());
        let _ = tx.send(result);
    });
}

pub fn poll_load(app: &mut App) {
    let received = app
        .ignore_list
        .load_rx
        .as_mut()
        .and_then(|rx| rx.try_recv().ok());
    if let Some(result) = received {
        app.ignore_list.load_rx = None;
        match result {
            Ok(users) => {
                if !users.is_empty() && app.ignore_list.selected >= users.len() {
                    app.ignore_list.selected = users.len() - 1;
                }
                app.ignore_list.users = users;
                app.ignore_list.error = None;
            }
            Err(e) => {
                app.ignore_list.error = Some(e);
            }
        }
        app.ignore_list.loading = false;
    }
}

// ---------------------------------------------------------------------------
// Draw
// ---------------------------------------------------------------------------

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    if app.ignore_list.loading {
        f.render_widget(
            Paragraph::new("Loading…")
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    // Area: list + optional add prompt row + optional error row.
    let has_prompt = app.ignore_list.add_prompt.is_some();
    let has_error = app.ignore_list.error.is_some();
    let extra = has_prompt as u16 + has_error as u16;
    let chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(extra),
    ])
    .split(area);
    let list_area = chunks[0];
    let bottom_area = if extra > 0 { Some(chunks[1]) } else { None };

    if app.ignore_list.users.is_empty() {
        f.render_widget(
            Paragraph::new("No ignored users.\n\nPress 'a' to ignore a user.")
                .style(Style::default().fg(MUTED))
                .alignment(Alignment::Center),
            list_area,
        );
    } else {
        let items: Vec<ListItem> = app
            .ignore_list
            .users
            .iter()
            .map(|u| {
                ListItem::new(Span::styled(
                    u.clone(),
                    Style::default().fg(ratatui::style::Color::White),
                ))
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .title(Span::styled(
                        format!(" {} ignored user(s) ", app.ignore_list.users.len()),
                        Style::default().fg(ACCENT),
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ACCENT)),
            )
            .highlight_style(
                Style::default()
                    .bg(ratatui::style::Color::Rgb(40, 60, 80))
                    .fg(ACCENT_DIM)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");

        let mut state = ListState::default();
        state.select(Some(app.ignore_list.selected));
        f.render_stateful_widget(list, list_area, &mut state);
    }

    if let Some(ba) = bottom_area {
        let mut sub = ba;
        if has_prompt {
            let row = Rect::new(sub.x, sub.y, sub.width, 1);
            let prompt_text = app.ignore_list.add_prompt.as_deref().unwrap_or("");
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(" Ignore user: ", Style::default().fg(ACCENT_DIM)),
                    Span::styled(
                        prompt_text.to_owned(),
                        Style::default().fg(ratatui::style::Color::White),
                    ),
                    Span::styled("█", Style::default().fg(ACCENT_DIM)),
                ])),
                row,
            );
            sub = Rect::new(sub.x, sub.y + 1, sub.width, sub.height.saturating_sub(1));
        }
        if has_error {
            if let Some(err) = &app.ignore_list.error {
                f.render_widget(
                    Paragraph::new(err.as_str())
                        .style(Style::default().fg(DANGER))
                        .alignment(Alignment::Center),
                    Rect::new(sub.x, sub.y, sub.width, 1),
                );
            }
        }
    }

    if app.ignore_list.confirm_unignore {
        draw_confirm(f, app);
    }
}

fn draw_confirm(f: &mut Frame, app: &App) {
    let user = app
        .ignore_list
        .users
        .get(app.ignore_list.selected)
        .map(|s| s.as_str())
        .unwrap_or("this user");

    let area = f.area();
    let popup = centered_rect(54, 7, area);
    f.render_widget(Clear, popup);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Unignore "),
            Span::styled(
                user.to_owned(),
                Style::default().fg(ACCENT_DIM).add_modifier(Modifier::BOLD),
            ),
            Span::raw("?"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  Enter/y",
                Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  confirm    "),
            Span::styled(
                "any other key",
                Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  cancel"),
        ]),
    ];

    f.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(Span::styled(
                        " Confirm ",
                        Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(DANGER))
                    .style(Style::default().bg(ratatui::style::Color::Rgb(25, 15, 15))),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}

pub fn hint_spans(app: &App) -> Vec<Span<'static>> {
    if app.ignore_list.add_prompt.is_some() {
        vec![
            Span::styled("Type", Style::default().fg(ACCENT_DIM)),
            Span::raw(" user ID  "),
            Span::styled("Enter", Style::default().fg(SUCCESS)),
            Span::raw(" submit  "),
            Span::styled("Esc", Style::default().fg(ACCENT)),
            Span::raw(" cancel"),
        ]
    } else {
        vec![
            Span::styled("j/k", Style::default().fg(ACCENT)),
            Span::raw(" navigate  "),
            Span::styled("a", Style::default().fg(ACCENT)),
            Span::raw(" add  "),
            Span::styled("d", Style::default().fg(DANGER)),
            Span::raw(" unignore  "),
            Span::styled("r", Style::default().fg(ACCENT)),
            Span::raw(" refresh  "),
            Span::styled(":", Style::default().fg(ACCENT)),
            Span::raw(" command  "),
            Span::styled("Esc/q", Style::default().fg(ACCENT)),
            Span::raw(" home"),
        ]
    }
}

pub fn tool_name() -> &'static str {
    "IgnoreList"
}
