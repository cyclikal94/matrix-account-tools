use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap},
};
use tokio::sync::oneshot;

use crate::app::{ActiveTool, App};
use crate::tools::{ACCENT, ACCENT_DIM, DANGER, MUTED, SUCCESS, FilterState, filter_hint_spans};
use crate::tools::common::{Cmd, handle_filter_keys, hint_spans_from_cmds, nav_down, nav_up};
use crate::ui::centered_rect;

const IGNORE_COLS: &[&str] = &["all"];

pub const CMDS: &[Cmd] = &[
    Cmd::new("j/k",    "navigate"),
    Cmd::new("a",      "add"),
    Cmd::danger("d",   "unignore"),
    Cmd::new("/",      "filter"),
    Cmd::new("r",      "refresh"),
    Cmd::new(":",      "command"),
    Cmd::new("Esc/q",  "home"),
];

pub const CMDS_ADD_PROMPT: &[Cmd] = &[
    Cmd::new("Type",     "user ID"),
    Cmd::success("Enter","submit"),
    Cmd::new("Esc",      "cancel"),
];

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct IgnoreListState {
    pub users: Vec<String>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub add_prompt: Option<String>,
    pub confirm_unignore: bool,
    pub load_rx: Option<oneshot::Receiver<Result<Vec<String>, String>>>,
    pub filter: FilterState,
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

    // Filter popup active.
    if app.ignore_list.filter.active {
        let filtered_len = filtered_users(app).len();
        handle_filter_keys(
            &mut app.ignore_list.filter,
            &mut app.ignore_list.selected,
            filtered_len,
            IGNORE_COLS.len(),
            code,
        );
        return;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
            if !app.ignore_list.filter.input.is_empty() {
                app.ignore_list.filter.clear();
            } else {
                app.active_tool = ActiveTool::Home;
            }
        }
        KeyCode::Char('/') => {
            app.ignore_list.filter.active = true;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let len = filtered_users(app).len();
            nav_down(&mut app.ignore_list.selected, len);
        }
        KeyCode::Char('k') | KeyCode::Up => nav_up(&mut app.ignore_list.selected),
        KeyCode::Char('a') | KeyCode::Char('A') => {
            app.ignore_list.add_prompt = Some(String::new());
            app.ignore_list.error = None;
        }
        KeyCode::Char('d') | KeyCode::Delete => {
            if !filtered_users(app).is_empty() {
                app.ignore_list.confirm_unignore = true;
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            start_load(app);
        }
        _ => {}
    }
}

fn filtered_users(app: &App) -> Vec<&String> {
    app.ignore_list
        .users
        .iter()
        .filter(|u| app.ignore_list.filter.matches(u))
        .collect()
}

async fn do_unignore(app: &mut App) {
    app.ignore_list.confirm_unignore = false;
    let users = filtered_users(app);
    let user_id = match users.get(app.ignore_list.selected) {
        Some(u) => (*u).clone(),
        None => return,
    };
    drop(users);
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

    let filtered: Vec<&String> = app
        .ignore_list
        .users
        .iter()
        .filter(|u| app.ignore_list.filter.matches(u))
        .collect();

    let total = app.ignore_list.users.len();
    let match_count = filtered.len();

    if filtered.is_empty() && app.ignore_list.users.is_empty() {
        f.render_widget(
            Paragraph::new("No ignored users.\n\nPress 'a' to ignore a user.")
                .style(Style::default().fg(MUTED))
                .alignment(Alignment::Center),
            list_area,
        );
    } else if filtered.is_empty() {
        f.render_widget(
            Paragraph::new("No matches.")
                .style(Style::default().fg(MUTED))
                .alignment(Alignment::Center),
            list_area,
        );
    } else {
        let items: Vec<ListItem> = filtered
            .iter()
            .map(|u| {
                ListItem::new(Span::styled(
                    (*u).clone(),
                    Style::default().fg(ratatui::style::Color::White),
                ))
            })
            .collect();

        let title = if !app.ignore_list.filter.input.is_empty() {
            format!(" Ignored Users ({match_count}/{total}) ")
        } else {
            format!(" Ignored Users ({total}) ")
        };

        let list = List::new(items)
            .block(
                Block::default()
                    .title(Span::styled(title, Style::default().fg(ACCENT)))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ACCENT))
                    .padding(Padding::new(1, 1, 1, 1)),
            )
            .highlight_style(
                Style::default()
                    .bg(ratatui::style::Color::Rgb(40, 60, 80))
                    .fg(ACCENT_DIM)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▌ ");

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

    if app.ignore_list.filter.active {
        crate::ui::draw_filter_popup(f, &app.ignore_list.filter, area);
    }
}

fn draw_confirm(f: &mut Frame, app: &App) {
    let users = filtered_users(app);
    let user = users
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
    if app.ignore_list.filter.active {
        return filter_hint_spans(app.ignore_list.filter.column, IGNORE_COLS);
    }
    if app.ignore_list.add_prompt.is_some() {
        return hint_spans_from_cmds(CMDS_ADD_PROMPT);
    }
    hint_spans_from_cmds(CMDS)
}
