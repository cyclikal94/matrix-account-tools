use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use crate::app::{ActiveTool, App, LoginState, Screen};
use crate::matrix::{AccountSummary, MatrixClient};
use crate::tools::{ACCENT, ERROR, FOCUSED, MUTED, SUCCESS};

#[derive(Debug, Default)]
pub struct AccountsToolState {
    pub accounts: Vec<AccountSummary>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
}

pub async fn handle(app: &mut App, code: KeyCode) {
    if app.accounts_tool.loading {
        return;
    }
    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
            app.active_tool = ActiveTool::Home;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if app.accounts_tool.selected + 1 < app.accounts_tool.accounts.len() {
                app.accounts_tool.selected += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.accounts_tool.selected > 0 {
                app.accounts_tool.selected -= 1;
            }
        }
        KeyCode::Enter => do_switch_account(app).await,
        KeyCode::Char('a') | KeyCode::Char('A') => {
            app.login = LoginState {
                can_go_back: true,
                ..LoginState::default()
            };
            app.screen = Screen::Login;
        }
        KeyCode::Char('d') | KeyCode::Delete => do_remove_account(app).await,
        _ => {}
    }
}

async fn do_switch_account(app: &mut App) {
    let user_id = match app.accounts_tool.accounts.get(app.accounts_tool.selected) {
        Some(a) => a.user_id.clone(),
        None => return,
    };
    if app.current_user_id.as_deref() == Some(&user_id) {
        return;
    }
    app.accounts_tool.loading = true;
    app.accounts_tool.error = None;

    match MatrixClient::restore_by_user_id(&user_id).await {
        Ok(Some(client)) => {
            if let Some(task) = app.sync_task.take() {
                task.abort();
            }
            app.sync_task = Some(client.start_background_sync());
            app.current_user_id = Some(client.user_id());
            app.matrix = Some(client);
            app.rooms_tool = crate::tools::rooms::RoomBrowserState::default();
        }
        Ok(None) => {
            app.accounts_tool.error = Some("Account not found.".to_owned());
        }
        Err(e) => {
            app.accounts_tool.error = Some(format!("Switch failed: {e}"));
        }
    }
    app.accounts_tool.loading = false;
    do_load_accounts(app).await;
}

async fn do_remove_account(app: &mut App) {
    let user_id = match app.accounts_tool.accounts.get(app.accounts_tool.selected) {
        Some(a) => a.user_id.clone(),
        None => return,
    };
    app.accounts_tool.loading = true;

    match MatrixClient::remove_account(&user_id).await {
        Ok(()) => {
            if app.current_user_id.as_deref() == Some(&user_id) {
                if let Some(task) = app.sync_task.take() {
                    task.abort();
                }
                app.matrix = None;
                app.current_user_id = None;
                match MatrixClient::restore_current().await {
                    Ok(Some(client)) => {
                        app.sync_task = Some(client.start_background_sync());
                        app.current_user_id = Some(client.user_id());
                        app.matrix = Some(client);
                    }
                    _ => {
                        app.accounts_tool.loading = false;
                        app.screen = Screen::Login;
                        return;
                    }
                }
            }
            app.rooms_tool = crate::tools::rooms::RoomBrowserState::default();
        }
        Err(e) => {
            app.accounts_tool.error = Some(format!("Remove failed: {e}"));
        }
    }
    app.accounts_tool.loading = false;
    do_load_accounts(app).await;
}

pub async fn do_load_accounts(app: &mut App) {
    match MatrixClient::list_accounts(app.current_user_id.as_deref()).await {
        Ok(accounts) => {
            app.accounts_tool.accounts = accounts;
            app.accounts_tool.error = None;
            if !app.accounts_tool.accounts.is_empty()
                && app.accounts_tool.selected >= app.accounts_tool.accounts.len()
            {
                app.accounts_tool.selected = app.accounts_tool.accounts.len() - 1;
            }
        }
        Err(e) => {
            app.accounts_tool.error = Some(format!("{e}"));
        }
    }
    app.accounts_tool.loading = false;
}

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    if app.accounts_tool.loading {
        f.render_widget(
            Paragraph::new("Loading accounts…")
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    if app.accounts_tool.accounts.is_empty() {
        f.render_widget(
            Paragraph::new("No accounts saved. Press 'a' to add one.")
                .style(Style::default().fg(MUTED))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = app
        .accounts_tool
        .accounts
        .iter()
        .map(|a| {
            let hs = a
                .homeserver
                .trim_end_matches('/')
                .trim_start_matches("https://")
                .trim_start_matches("http://");
            let marker = if a.is_current {
                Span::styled(" ✓", Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD))
            } else {
                Span::raw("")
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    a.user_id.clone(),
                    Style::default()
                        .fg(if a.is_current {
                            SUCCESS
                        } else {
                            ratatui::style::Color::White
                        })
                        .add_modifier(if a.is_current {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                marker,
                Span::styled(format!("  {hs}"), Style::default().fg(MUTED)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(Span::styled(
                    format!(" {} account(s) ", app.accounts_tool.accounts.len()),
                    Style::default().fg(ACCENT),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ACCENT)),
        )
        .highlight_style(
            Style::default()
                .bg(ratatui::style::Color::Rgb(40, 60, 80))
                .fg(FOCUSED)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    state.select(Some(app.accounts_tool.selected));

    if let Some(err) = &app.accounts_tool.error {
        let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
        f.render_stateful_widget(list, chunks[0], &mut state);
        f.render_widget(
            Paragraph::new(err.as_str())
                .style(Style::default().fg(ERROR))
                .alignment(Alignment::Center),
            chunks[1],
        );
    } else {
        f.render_stateful_widget(list, area, &mut state);
    }
}

pub fn hint_spans() -> Vec<Span<'static>> {
    vec![
        Span::styled("j/k", Style::default().fg(ACCENT)),
        Span::raw(" navigate  "),
        Span::styled("Enter", Style::default().fg(ACCENT)),
        Span::raw(" switch  "),
        Span::styled("a", Style::default().fg(ACCENT)),
        Span::raw(" add  "),
        Span::styled("d", Style::default().fg(ERROR)),
        Span::raw(" remove  "),
        Span::styled(":", Style::default().fg(ACCENT)),
        Span::raw(" command  "),
        Span::styled("Esc/q", Style::default().fg(ACCENT)),
        Span::raw(" home"),
    ]
}

pub fn tool_name() -> &'static str {
    "Accounts"
}
