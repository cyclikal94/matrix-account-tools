use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::ListItem,
};

use crate::app::{ActiveTool, App, LoginState, Screen};
use crate::matrix::{AccountSummary, MatrixClient};
use crate::tools::{ACCENT, BORDER, FG, MUTED, FilterState, Filterable, filter_hint_spans};
use crate::tools::common::{Cmd, draw_list_block, handle_filter_keys, hint_spans_from_cmds, nav_down, nav_up};

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

pub const CMDS: &[Cmd] = &[
    Cmd::new("j/k",    "navigate"),
    Cmd::new("Enter",  "switch"),
    Cmd::new("a",      "add"),
    Cmd::danger("d",   "remove"),
    Cmd::new("/",      "filter"),
    Cmd::new(":",      "command"),
    Cmd::new("Esc/q",  "home"),
];

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

    if app.accounts_tool.filter.active {
        let filtered_len = filtered_accounts(app).len();
        handle_filter_keys(
            &mut app.accounts_tool.filter,
            &mut app.accounts_tool.selected,
            filtered_len,
            AccountSummary::filter_cols().len(),
            code,
        );
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
            nav_down(&mut app.accounts_tool.selected, len);
        }
        KeyCode::Char('k') | KeyCode::Up => nav_up(&mut app.accounts_tool.selected),
        KeyCode::Enter => do_switch_account(app).await,
        KeyCode::Char('a') | KeyCode::Char('A') => {
            app.login = LoginState { can_go_back: true, ..LoginState::default() };
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
    draw_list_block(
        f,
        "Accounts",
        &app.accounts_tool.accounts,
        app.accounts_tool.selected,
        &app.accounts_tool.filter,
        app.accounts_tool.loading,
        true,
        &app.accounts_tool.error,
        area,
        "Loading accounts…",
        "No accounts saved. Press 'a' to add one.",
        |a: &AccountSummary| {
            let hs = a.homeserver
                .trim_end_matches('/')
                .trim_start_matches("https://")
                .trim_start_matches("http://");
            let avatar_char = a.user_id
                .trim_start_matches('@')
                .chars().next().unwrap_or('?')
                .to_ascii_uppercase();
            let active_badge = if a.is_current {
                Span::styled("  ● active", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
            } else {
                Span::raw("")
            };
            ListItem::new(Line::from(vec![
                Span::styled("[",  Style::default().fg(BORDER)),
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
        },
    );

    if app.accounts_tool.filter.active {
        crate::ui::draw_filter_popup(f, &app.accounts_tool.filter, area);
    }
}

pub fn hint_spans(app: &App) -> Vec<Span<'static>> {
    if app.accounts_tool.filter.active {
        return filter_hint_spans(app.accounts_tool.filter.column, AccountSummary::filter_cols());
    }
    hint_spans_from_cmds(CMDS)
}
