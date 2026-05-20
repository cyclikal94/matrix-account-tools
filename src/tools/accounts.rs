use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Padding, Paragraph},
};

use crate::app::{ActiveTool, App, LoginState, Screen};
use crate::matrix::{AccountSummary, MatrixClient};
use crate::tools::{ACCENT, ACCENT_DIM, BG3, BORDER, DANGER, FG, MUTED, FilterState, Filterable, filter_hint_spans};

impl Filterable for AccountSummary {
    fn filter_cols() -> &'static [&'static str] { &["all", "id", "server"] }
    fn filter_value(&self, col: usize) -> String {
        match col {
            1 => self.user_id.clone(),
            2 => self.homeserver.trim_end_matches('/').trim_start_matches("https://").trim_start_matches("http://").to_owned(),
            _ => String::new(),
        }
    }
}

#[derive(Debug, Default)]
pub struct AccountsToolState {
    pub accounts: Vec<AccountSummary>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub filter: FilterState,
}

fn filtered_accounts(app: &App) -> Vec<&AccountSummary> {
    app.accounts_tool.accounts.iter()
        .filter(|a| app.accounts_tool.filter.matches_item(*a))
        .collect()
}

pub async fn handle(app: &mut App, code: KeyCode) {
    if app.accounts_tool.loading {
        return;
    }

    // Filter popup active — intercept keys.
    if app.accounts_tool.filter.active {
        match code {
            KeyCode::Esc => app.accounts_tool.filter.clear(),
            KeyCode::Enter => app.accounts_tool.filter.active = false,
            KeyCode::Backspace => {
                app.accounts_tool.filter.input.pop();
                app.accounts_tool.selected = 0;
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let n = c.to_digit(10).unwrap() as usize;
                if n < AccountSummary::filter_cols().len() {
                    app.accounts_tool.filter.column = if n == 0 { None } else { Some(n) };
                    app.accounts_tool.selected = 0;
                }
            }
            KeyCode::Char(c) if !c.is_control() => {
                app.accounts_tool.filter.input.push(c);
                app.accounts_tool.selected = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let len = filtered_accounts(app).len();
                if app.accounts_tool.selected + 1 < len {
                    app.accounts_tool.selected += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if app.accounts_tool.selected > 0 {
                    app.accounts_tool.selected -= 1;
                }
            }
            _ => {}
        }
        return;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
            if !app.accounts_tool.filter.input.is_empty() {
                app.accounts_tool.filter.clear();
            } else {
                app.active_tool = ActiveTool::Home;
            }
        }
        KeyCode::Char('/') => {
            app.accounts_tool.filter.active = true;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let len = filtered_accounts(app).len();
            if app.accounts_tool.selected + 1 < len {
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
    let accounts = filtered_accounts(app);
    let user_id = match accounts.get(app.accounts_tool.selected) {
        Some(a) => a.user_id.clone(),
        None => return,
    };
    drop(accounts);
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
    let accounts = filtered_accounts(app);
    let user_id = match accounts.get(app.accounts_tool.selected) {
        Some(a) => a.user_id.clone(),
        None => return,
    };
    drop(accounts);
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

    let filtered: Vec<&AccountSummary> = app
        .accounts_tool
        .accounts
        .iter()
        .filter(|a| {
            let hs = a
                .homeserver
                .trim_end_matches('/')
                .trim_start_matches("https://")
                .trim_start_matches("http://");
            app.accounts_tool.filter.matches(&a.user_id)
                || app.accounts_tool.filter.matches(hs)
        })
        .collect();

    let total = app.accounts_tool.accounts.len();
    let match_count = filtered.len();

    let items: Vec<ListItem> = filtered
        .iter()
        .map(|a| {
            let hs = a
                .homeserver
                .trim_end_matches('/')
                .trim_start_matches("https://")
                .trim_start_matches("http://");
            let avatar_char = a
                .user_id
                .trim_start_matches('@')
                .chars()
                .next()
                .unwrap_or('?')
                .to_ascii_uppercase();
            let active_badge = if a.is_current {
                Span::styled("  ● active", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
            } else {
                Span::raw("")
            };
            ListItem::new(Line::from(vec![
                Span::styled("[", Style::default().fg(BORDER)),
                Span::styled(avatar_char.to_string(), Style::default().fg(MUTED)),
                Span::styled("] ", Style::default().fg(BORDER)),
                Span::styled(
                    a.user_id.clone(),
                    Style::default()
                        .fg(if a.is_current { FG } else { MUTED })
                        .add_modifier(if a.is_current { Modifier::BOLD } else { Modifier::empty() }),
                ),
                Span::styled(format!("  {hs}"), Style::default().fg(MUTED)),
                active_badge,
            ]))
        })
        .collect();

    let title = if !app.accounts_tool.filter.input.is_empty() {
        format!(" {match_count} / {total} accounts ")
    } else {
        format!(" {total} account(s) ")
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(Span::styled(title, Style::default().fg(MUTED)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER))
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
    state.select(Some(app.accounts_tool.selected));

    if let Some(err) = &app.accounts_tool.error {
        let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
        f.render_stateful_widget(list, chunks[0], &mut state);
        f.render_widget(
            Paragraph::new(err.as_str())
                .style(Style::default().fg(DANGER))
                .alignment(Alignment::Center),
            chunks[1],
        );
    } else {
        f.render_stateful_widget(list, area, &mut state);
    }

    if app.accounts_tool.filter.active {
        crate::ui::draw_filter_popup(f, &app.accounts_tool.filter, match_count, total, area);
    }
}

pub fn hint_spans(app: &App) -> Vec<Span<'static>> {
    if app.accounts_tool.filter.active {
        return filter_hint_spans(app.accounts_tool.filter.column, AccountSummary::filter_cols());
    }
    vec![
        Span::styled("j/k", Style::default().fg(ACCENT)),
        Span::raw(" navigate  "),
        Span::styled("Enter", Style::default().fg(ACCENT)),
        Span::raw(" switch  "),
        Span::styled("a", Style::default().fg(ACCENT)),
        Span::raw(" add  "),
        Span::styled("d", Style::default().fg(DANGER)),
        Span::raw(" remove  "),
        Span::styled("/", Style::default().fg(ACCENT)),
        Span::raw(" filter  "),
        Span::styled(":", Style::default().fg(ACCENT)),
        Span::raw(" command  "),
        Span::styled("Esc/q", Style::default().fg(ACCENT)),
        Span::raw(" home"),
    ]
}
