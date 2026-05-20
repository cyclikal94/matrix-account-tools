use std::time::Instant;

use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap},
};
use tokio::sync::oneshot;

use crate::app::{ActiveTool, App, LoginState, Screen};
use crate::matrix::{AccountSummary, DeviceInfo, MatrixClient};
use crate::tools::{
    ACCENT, ACCENT_DIM, BG, BG3, BORDER, DANGER, FG, FG2, MUTED, MUTED2, SUCCESS,
    FilterState, Filterable, filter_hint_spans,
};
use crate::tools::common::{
    Cmd, draw_confirm_popup, handle_confirm_key, handle_filter_keys,
    hint_spans_from_cmds, nav_down, nav_up,
};
use crate::ui::centered_rect;

// ---------------------------------------------------------------------------
// Filterable
// ---------------------------------------------------------------------------

impl Filterable for AccountSummary {
    fn filter_cols() -> &'static [&'static str] { &["all", "id", "server"] }
    fn filter_value(&self, col: usize) -> String {
        match col {
            1 => self.user_id.clone(),
            2 => self.homeserver
                .trim_end_matches('/')
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .to_owned(),
            _ => String::new(),
        }
    }
}

impl Filterable for DeviceInfo {
    fn filter_cols() -> &'static [&'static str] { &["all", "name", "id", "ip"] }
    fn filter_value(&self, col: usize) -> String {
        match col {
            1 => self.display_name.clone().unwrap_or_default(),
            2 => self.device_id.clone(),
            3 => self.last_seen_ip.clone().unwrap_or_default(),
            _ => String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum AccountTab {
    #[default]
    Devices,
    IgnoredUsers,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ProfileField {
    #[default]
    DisplayName,
    AvatarUrl,
}

#[derive(Debug)]
pub enum DeviceDeleteDialog {
    Confirm,
    EnterPassword(String),
}

// ---------------------------------------------------------------------------
// CMDS
// ---------------------------------------------------------------------------

pub const CMDS_LIST: &[Cmd] = &[
    Cmd::new("j/k",   "navigate"),
    Cmd::new("Enter", "switch"),
    Cmd::new("d",     "detail"),
    Cmd::new("v",     "devices"),
    Cmd::new("i",     "ignored"),
    Cmd::new("a",     "add"),
    Cmd::danger("x",  "remove"),
    Cmd::new("/",     "filter"),
    Cmd::new(":",     "command"),
    Cmd::new("Esc/q", "home"),
];

pub const CMDS_DETAIL: &[Cmd] = &[
    Cmd::new("j/k",     "navigate"),
    Cmd::new("e/Enter", "edit"),
    Cmd::new("v",       "devices"),
    Cmd::new("i",       "ignored"),
    Cmd::new("r",       "reload"),
    Cmd::new(":",       "command"),
    Cmd::new("Esc/q",   "back"),
];

pub const CMDS_EDITING: &[Cmd] = &[
    Cmd::success("Enter", "save"),
    Cmd::new("Esc",       "discard"),
];

pub const CMDS_DEVICES: &[Cmd] = &[
    Cmd::new("j/k",   "navigate"),
    Cmd::danger("x",  "sign out"),
    Cmd::new("/",     "filter"),
    Cmd::new("r",     "refresh"),
    Cmd::new("d",     "detail"),
    Cmd::new("i",     "ignored"),
    Cmd::new("Esc",   "back"),
];

pub const CMDS_IGNORED: &[Cmd] = &[
    Cmd::new("j/k",   "navigate"),
    Cmd::new("a",     "add"),
    Cmd::danger("x",  "unignore"),
    Cmd::new("/",     "filter"),
    Cmd::new("r",     "refresh"),
    Cmd::new("d",     "detail"),
    Cmd::new("v",     "devices"),
    Cmd::new("Esc",   "back"),
];

pub const CMDS_IGNORED_ADD: &[Cmd] = &[
    Cmd::new("Type",      "user ID"),
    Cmd::success("Enter", "submit"),
    Cmd::new("Esc",       "cancel"),
];

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct AccountsToolState {
    // List panel
    pub accounts: Vec<AccountSummary>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub filter: FilterState,

    // Right panel focus
    pub detail_open: bool,
    pub detail_tab_focused: bool,

    // Profile detail (always for the active/logged-in account)
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub edit_display_name: Option<String>,
    pub edit_avatar_url: Option<String>,
    pub profile_field: ProfileField,
    pub profile_loading: bool,
    pub profile_saving: bool,
    pub profile_error: Option<String>,
    pub profile_load_rx: Option<oneshot::Receiver<Result<(Option<String>, Option<String>), String>>>,

    // Tabs
    pub active_tab: AccountTab,

    // Devices tab
    pub devices: Vec<DeviceInfo>,
    pub devices_selected: usize,
    pub devices_loading: bool,
    pub devices_error: Option<String>,
    pub devices_filter: FilterState,
    pub devices_load_rx: Option<oneshot::Receiver<Result<Vec<DeviceInfo>, String>>>,
    pub delete_dialog: Option<(String, DeviceDeleteDialog)>,

    // Ignored Users tab
    pub ignored_users: Vec<String>,
    pub ignored_selected: usize,
    pub ignored_loading: bool,
    pub ignored_error: Option<String>,
    pub ignored_filter: FilterState,
    pub ignored_load_rx: Option<oneshot::Receiver<Result<Vec<String>, String>>>,
    pub ignored_add_prompt: Option<String>,
    pub ignored_confirm_unignore: bool,

    // Account removal confirmation
    pub confirm_remove: bool,
}

impl Default for AccountsToolState {
    fn default() -> Self {
        Self {
            accounts: Vec::new(),
            selected: 0,
            loading: false,
            error: None,
            filter: FilterState::default(),
            detail_open: false,
            detail_tab_focused: false,
            display_name: None,
            avatar_url: None,
            edit_display_name: None,
            edit_avatar_url: None,
            profile_field: ProfileField::default(),
            profile_loading: false,
            profile_saving: false,
            profile_error: None,
            profile_load_rx: None,
            active_tab: AccountTab::default(),
            devices: Vec::new(),
            devices_selected: 0,
            devices_loading: false,
            devices_error: None,
            devices_filter: FilterState::default(),
            devices_load_rx: None,
            delete_dialog: None,
            ignored_users: Vec::new(),
            ignored_selected: 0,
            ignored_loading: false,
            ignored_error: None,
            ignored_filter: FilterState::default(),
            ignored_load_rx: None,
            ignored_add_prompt: None,
            ignored_confirm_unignore: false,
            confirm_remove: false,
        }
    }
}

impl AccountsToolState {
    pub fn is_profile_editing(&self) -> bool {
        self.edit_display_name.is_some() || self.edit_avatar_url.is_some()
    }

    fn active_profile_edit(&mut self) -> Option<&mut String> {
        match self.profile_field {
            ProfileField::DisplayName => self.edit_display_name.as_mut(),
            ProfileField::AvatarUrl => self.edit_avatar_url.as_mut(),
        }
    }
}

fn filtered_accounts(app: &App) -> Vec<&AccountSummary> {
    app.accounts_tool.accounts.iter()
        .filter(|a| app.accounts_tool.filter.matches_item(*a))
        .collect()
}

fn filtered_devices(app: &App) -> Vec<&DeviceInfo> {
    app.accounts_tool.devices.iter()
        .filter(|d| app.accounts_tool.devices_filter.matches_item(*d))
        .collect()
}

fn filtered_ignored(app: &App) -> Vec<&String> {
    app.accounts_tool.ignored_users.iter()
        .filter(|u| app.accounts_tool.ignored_filter.matches(u))
        .collect()
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn handle(app: &mut App, code: KeyCode) {
    if app.accounts_tool.delete_dialog.is_some() {
        handle_delete_dialog(app, code).await;
        return;
    }
    if app.accounts_tool.confirm_remove {
        handle_remove_confirm(app, code).await;
        return;
    }
    if app.accounts_tool.ignored_confirm_unignore {
        handle_ignored_confirm(app, code).await;
        return;
    }
    if app.accounts_tool.ignored_add_prompt.is_some() {
        handle_ignored_add_prompt(app, code).await;
        return;
    }

    if app.accounts_tool.detail_open {
        if app.accounts_tool.detail_tab_focused {
            match app.accounts_tool.active_tab {
                AccountTab::Devices => handle_devices_tab(app, code).await,
                AccountTab::IgnoredUsers => handle_ignored_tab(app, code).await,
            }
        } else {
            handle_detail(app, code).await;
        }
    } else {
        handle_list(app, code).await;
    }
}

async fn handle_list(app: &mut App, code: KeyCode) {
    if app.accounts_tool.loading { return; }

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
        KeyCode::Char('/') => app.accounts_tool.filter.active = true,
        KeyCode::Char('j') | KeyCode::Down => {
            let len = filtered_accounts(app).len();
            nav_down(&mut app.accounts_tool.selected, len);
        }
        KeyCode::Char('k') | KeyCode::Up => nav_up(&mut app.accounts_tool.selected),
        KeyCode::Enter => {
            do_switch_account(app).await;
        }
        KeyCode::Char('d') | KeyCode::Char('e') => {
            app.accounts_tool.detail_open = true;
            app.accounts_tool.detail_tab_focused = false;
        }
        KeyCode::Char('v') => {
            app.accounts_tool.detail_open = true;
            app.accounts_tool.detail_tab_focused = true;
            app.accounts_tool.active_tab = AccountTab::Devices;
        }
        KeyCode::Char('i') => {
            app.accounts_tool.detail_open = true;
            app.accounts_tool.detail_tab_focused = true;
            app.accounts_tool.active_tab = AccountTab::IgnoredUsers;
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            app.login = LoginState { can_go_back: true, ..LoginState::default() };
            app.screen = Screen::Login;
        }
        KeyCode::Char('x') | KeyCode::Char('X') | KeyCode::Delete => {
            if !filtered_accounts(app).is_empty() {
                app.accounts_tool.confirm_remove = true;
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.accounts_tool.loading = true;
            do_load_accounts(app).await;
        }
        _ => {}
    }
}

async fn handle_detail(app: &mut App, code: KeyCode) {
    if app.accounts_tool.profile_saving { return; }

    if app.accounts_tool.is_profile_editing() {
        match code {
            KeyCode::Esc => {
                app.accounts_tool.edit_display_name = None;
                app.accounts_tool.edit_avatar_url = None;
            }
            KeyCode::Backspace => {
                if let Some(s) = app.accounts_tool.active_profile_edit() {
                    s.pop();
                }
            }
            KeyCode::Char(c) if !c.is_control() => {
                if let Some(s) = app.accounts_tool.active_profile_edit() {
                    s.push(c);
                }
            }
            KeyCode::Enter => do_save_profile(app).await,
            _ => {}
        }
        return;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
            app.accounts_tool.detail_open = false;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.accounts_tool.profile_field = ProfileField::AvatarUrl;
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.accounts_tool.profile_field = ProfileField::DisplayName;
        }
        KeyCode::Char('e') | KeyCode::Enter => {
            let current = match app.accounts_tool.profile_field {
                ProfileField::DisplayName => app.accounts_tool.display_name.clone(),
                ProfileField::AvatarUrl => app.accounts_tool.avatar_url.clone(),
            };
            match app.accounts_tool.profile_field {
                ProfileField::DisplayName => {
                    app.accounts_tool.edit_display_name = Some(current.unwrap_or_default());
                }
                ProfileField::AvatarUrl => {
                    app.accounts_tool.edit_avatar_url = Some(current.unwrap_or_default());
                }
            }
            app.accounts_tool.profile_error = None;
        }
        KeyCode::Char('v') => {
            app.accounts_tool.detail_tab_focused = true;
            app.accounts_tool.active_tab = AccountTab::Devices;
        }
        KeyCode::Char('i') => {
            app.accounts_tool.detail_tab_focused = true;
            app.accounts_tool.active_tab = AccountTab::IgnoredUsers;
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            start_profile_load(app);
        }
        _ => {}
    }
}

async fn handle_devices_tab(app: &mut App, code: KeyCode) {
    if app.accounts_tool.devices_loading { return; }

    if app.accounts_tool.devices_filter.active {
        let filtered_len = filtered_devices(app).len();
        handle_filter_keys(
            &mut app.accounts_tool.devices_filter,
            &mut app.accounts_tool.devices_selected,
            filtered_len,
            DeviceInfo::filter_cols().len(),
            code,
        );
        return;
    }

    match code {
        KeyCode::Esc => {
            if !app.accounts_tool.devices_filter.input.is_empty() {
                app.accounts_tool.devices_filter.clear();
            } else {
                app.accounts_tool.detail_tab_focused = false;
                app.accounts_tool.detail_open = false;
            }
        }
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            app.accounts_tool.detail_tab_focused = false;
            app.accounts_tool.detail_open = false;
        }
        KeyCode::Char('d') => {
            app.accounts_tool.detail_tab_focused = false;
        }
        KeyCode::Char('i') => {
            app.accounts_tool.active_tab = AccountTab::IgnoredUsers;
            app.accounts_tool.devices_filter.clear();
        }
        KeyCode::Char('/') => app.accounts_tool.devices_filter.active = true,
        KeyCode::Char('j') | KeyCode::Down => {
            let len = filtered_devices(app).len();
            nav_down(&mut app.accounts_tool.devices_selected, len);
        }
        KeyCode::Char('k') | KeyCode::Up => nav_up(&mut app.accounts_tool.devices_selected),
        KeyCode::Char('x') | KeyCode::Char('X') | KeyCode::Delete => {
            let devs = filtered_devices(app);
            if let Some(dev) = devs.get(app.accounts_tool.devices_selected) {
                if dev.is_current {
                    app.accounts_tool.devices_error =
                        Some("Cannot sign out the current device.".to_owned());
                } else {
                    let id = dev.device_id.clone();
                    drop(devs);
                    app.accounts_tool.delete_dialog =
                        Some((id, DeviceDeleteDialog::Confirm));
                    app.accounts_tool.devices_error = None;
                }
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') => start_devices_load(app),
        _ => {}
    }
}

async fn handle_ignored_tab(app: &mut App, code: KeyCode) {
    if app.accounts_tool.ignored_loading { return; }

    if app.accounts_tool.ignored_filter.active {
        let filtered_len = filtered_ignored(app).len();
        handle_filter_keys(
            &mut app.accounts_tool.ignored_filter,
            &mut app.accounts_tool.ignored_selected,
            filtered_len,
            1, // ignored users only have "all" column
            code,
        );
        return;
    }

    match code {
        KeyCode::Esc => {
            if !app.accounts_tool.ignored_filter.input.is_empty() {
                app.accounts_tool.ignored_filter.clear();
            } else {
                app.accounts_tool.detail_tab_focused = false;
                app.accounts_tool.detail_open = false;
            }
        }
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            app.accounts_tool.detail_tab_focused = false;
            app.accounts_tool.detail_open = false;
        }
        KeyCode::Char('d') => {
            app.accounts_tool.detail_tab_focused = false;
        }
        KeyCode::Char('v') => {
            app.accounts_tool.active_tab = AccountTab::Devices;
            app.accounts_tool.ignored_filter.clear();
        }
        KeyCode::Char('/') => app.accounts_tool.ignored_filter.active = true,
        KeyCode::Char('j') | KeyCode::Down => {
            let len = filtered_ignored(app).len();
            nav_down(&mut app.accounts_tool.ignored_selected, len);
        }
        KeyCode::Char('k') | KeyCode::Up => nav_up(&mut app.accounts_tool.ignored_selected),
        KeyCode::Char('a') | KeyCode::Char('A') => {
            app.accounts_tool.ignored_add_prompt = Some(String::new());
            app.accounts_tool.ignored_error = None;
        }
        KeyCode::Char('x') | KeyCode::Char('X') | KeyCode::Delete => {
            if !filtered_ignored(app).is_empty() {
                app.accounts_tool.ignored_confirm_unignore = true;
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') => start_ignored_load(app),
        _ => {}
    }
}

async fn handle_ignored_add_prompt(app: &mut App, code: KeyCode) {
    let input = match app.accounts_tool.ignored_add_prompt.clone() {
        Some(s) => s,
        None => return,
    };
    match code {
        KeyCode::Esc => app.accounts_tool.ignored_add_prompt = None,
        KeyCode::Backspace => {
            let mut s = input;
            s.pop();
            app.accounts_tool.ignored_add_prompt = Some(s);
        }
        KeyCode::Char(c) if !c.is_control() => {
            let mut s = input;
            s.push(c);
            app.accounts_tool.ignored_add_prompt = Some(s);
        }
        KeyCode::Enter => {
            let user_id = input.trim().to_owned();
            if !user_id.starts_with('@') || !user_id.contains(':') {
                app.accounts_tool.ignored_error =
                    Some("Invalid user ID — must be @user:server".to_owned());
                app.accounts_tool.ignored_add_prompt = None;
            } else {
                app.accounts_tool.ignored_add_prompt = None;
                if let Some(client) = &app.matrix {
                    match client.ignore_user(&user_id).await {
                        Ok(()) => app.accounts_tool.ignored_error = None,
                        Err(e) => app.accounts_tool.ignored_error = Some(format!("{e}")),
                    }
                }
                start_ignored_load(app);
            }
        }
        _ => {}
    }
}

async fn handle_ignored_confirm(app: &mut App, code: KeyCode) {
    app.accounts_tool.ignored_confirm_unignore = false;
    if handle_confirm_key(code) {
        let users = filtered_ignored(app);
        let user_id = match users.get(app.accounts_tool.ignored_selected) {
            Some(u) => (*u).clone(),
            None => return,
        };
        drop(users);
        if let Some(client) = &app.matrix {
            match client.unignore_user(&user_id).await {
                Ok(()) => app.accounts_tool.ignored_error = None,
                Err(e) => app.accounts_tool.ignored_error = Some(format!("{e}")),
            }
        }
        start_ignored_load(app);
    }
}

async fn handle_remove_confirm(app: &mut App, code: KeyCode) {
    app.accounts_tool.confirm_remove = false;
    if handle_confirm_key(code) {
        do_remove_account(app).await;
    }
}

async fn handle_delete_dialog(app: &mut App, code: KeyCode) {
    let Some((device_id, ref state)) = &app.accounts_tool.delete_dialog else { return; };
    let device_id = device_id.clone();
    match state {
        DeviceDeleteDialog::Confirm => match code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                app.accounts_tool.delete_dialog =
                    Some((device_id, DeviceDeleteDialog::EnterPassword(String::new())));
            }
            _ => app.accounts_tool.delete_dialog = None,
        },
        DeviceDeleteDialog::EnterPassword(ref pwd) => {
            let pwd = pwd.clone();
            match code {
                KeyCode::Esc => {
                    app.accounts_tool.delete_dialog =
                        Some((device_id, DeviceDeleteDialog::Confirm));
                }
                KeyCode::Backspace => {
                    let mut s = pwd;
                    s.pop();
                    app.accounts_tool.delete_dialog =
                        Some((device_id, DeviceDeleteDialog::EnterPassword(s)));
                }
                KeyCode::Char(c) if !c.is_control() => {
                    let mut s = pwd;
                    s.push(c);
                    app.accounts_tool.delete_dialog =
                        Some((device_id, DeviceDeleteDialog::EnterPassword(s)));
                }
                KeyCode::Enter => {
                    let password = pwd.clone();
                    app.accounts_tool.delete_dialog = None;
                    do_delete_device(app, &device_id, &password).await;
                }
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

async fn do_switch_account(app: &mut App) {
    let accounts = filtered_accounts(app);
    let user_id = match accounts.get(app.accounts_tool.selected) {
        Some(a) => a.user_id.clone(),
        None => return,
    };
    drop(accounts);
    if app.current_user_id.as_deref() == Some(&user_id) {
        app.accounts_tool.detail_open = true;
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
            // Reset detail data for the new account.
            app.accounts_tool.display_name = None;
            app.accounts_tool.avatar_url = None;
            app.accounts_tool.devices = Vec::new();
            app.accounts_tool.ignored_users = Vec::new();
            start_profile_load(app);
            start_devices_load(app);
            start_ignored_load(app);
        }
        Ok(None) => {
            app.accounts_tool.error = Some("Account not found.".to_owned());
        }
        Err(e) => {
            app.accounts_tool.error = Some(format!("Switch failed: {e}"));
        }
    }
    app.accounts_tool.loading = false;
    app.accounts_tool.detail_open = true;
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
                app.accounts_tool.display_name = None;
                app.accounts_tool.avatar_url = None;
                app.accounts_tool.devices = Vec::new();
                app.accounts_tool.ignored_users = Vec::new();
                match MatrixClient::restore_current().await {
                    Ok(Some(client)) => {
                        app.sync_task = Some(client.start_background_sync());
                        app.current_user_id = Some(client.user_id());
                        app.matrix = Some(client);
                        start_profile_load(app);
                        start_devices_load(app);
                        start_ignored_load(app);
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

async fn do_save_profile(app: &mut App) {
    app.accounts_tool.profile_saving = true;
    app.accounts_tool.profile_error = None;

    let Some(client) = &app.matrix else {
        app.accounts_tool.profile_saving = false;
        app.accounts_tool.profile_error = Some("Not connected.".to_owned());
        app.accounts_tool.edit_display_name = None;
        app.accounts_tool.edit_avatar_url = None;
        return;
    };

    let result = match app.accounts_tool.profile_field {
        ProfileField::DisplayName => {
            let val = app.accounts_tool.edit_display_name.take().unwrap_or_default();
            let v = if val.is_empty() { None } else { Some(val.as_str()) };
            let r = client.set_display_name(v).await;
            if r.is_ok() {
                app.accounts_tool.display_name = if val.is_empty() { None } else { Some(val) };
            }
            r
        }
        ProfileField::AvatarUrl => {
            let val = app.accounts_tool.edit_avatar_url.take().unwrap_or_default();
            if !val.is_empty() && !val.starts_with("mxc://") {
                app.accounts_tool.profile_error =
                    Some("Avatar URL must start with mxc://".to_owned());
                app.accounts_tool.profile_saving = false;
                return;
            }
            let v = if val.is_empty() { None } else { Some(val.as_str()) };
            let r = client.set_avatar_url(v).await;
            if r.is_ok() {
                app.accounts_tool.avatar_url = if val.is_empty() { None } else { Some(val) };
            }
            r
        }
    };

    match result {
        Ok(()) => app.toast = Some(("Saved!".to_owned(), SUCCESS, Instant::now())),
        Err(e) => app.accounts_tool.profile_error = Some(format!("{e}")),
    }
    app.accounts_tool.profile_saving = false;
}

async fn do_delete_device(app: &mut App, device_id: &str, password: &str) {
    if let Some(client) = &app.matrix {
        match client.delete_device(device_id, password).await {
            Ok(()) => app.accounts_tool.devices_error = None,
            Err(e) => app.accounts_tool.devices_error = Some(format!("Sign out failed: {e}")),
        }
    }
    start_devices_load(app);
}

pub fn start_profile_load(app: &mut App) {
    let Some(client) = app.matrix.clone() else { return; };
    app.accounts_tool.profile_loading = true;
    app.accounts_tool.profile_error = None;
    let (tx, rx) = oneshot::channel();
    app.accounts_tool.profile_load_rx = Some(rx);
    tokio::spawn(async move {
        let result = client.get_profile().await.map_err(|e| e.to_string());
        let _ = tx.send(result);
    });
}

pub fn start_devices_load(app: &mut App) {
    let Some(client) = app.matrix.clone() else { return; };
    app.accounts_tool.devices_loading = true;
    app.accounts_tool.devices_error = None;
    let (tx, rx) = oneshot::channel();
    app.accounts_tool.devices_load_rx = Some(rx);
    tokio::spawn(async move {
        let result = client.get_devices().await.map_err(|e| e.to_string());
        let _ = tx.send(result);
    });
}

pub fn start_ignored_load(app: &mut App) {
    let Some(client) = app.matrix.clone() else { return; };
    app.accounts_tool.ignored_loading = true;
    app.accounts_tool.ignored_error = None;
    let (tx, rx) = oneshot::channel();
    app.accounts_tool.ignored_load_rx = Some(rx);
    tokio::spawn(async move {
        let result = client.get_ignored_users().await.map_err(|e| e.to_string());
        let _ = tx.send(result);
    });
}

// ---------------------------------------------------------------------------
// Poll (called each frame from run loop)
// ---------------------------------------------------------------------------

pub fn poll(app: &mut App) {
    poll_profile(app);
    poll_devices(app);
    poll_ignored(app);
}

fn poll_profile(app: &mut App) {
    let received = app.accounts_tool.profile_load_rx
        .as_mut()
        .and_then(|rx| rx.try_recv().ok());
    if let Some(result) = received {
        app.accounts_tool.profile_load_rx = None;
        app.accounts_tool.profile_loading = false;
        match result {
            Ok((dn, av)) => {
                app.accounts_tool.display_name = dn;
                app.accounts_tool.avatar_url = av;
                app.accounts_tool.profile_error = None;
            }
            Err(e) => app.accounts_tool.profile_error = Some(e),
        }
    }
}

fn poll_devices(app: &mut App) {
    let received = app.accounts_tool.devices_load_rx
        .as_mut()
        .and_then(|rx| rx.try_recv().ok());
    if let Some(result) = received {
        app.accounts_tool.devices_load_rx = None;
        app.accounts_tool.devices_loading = false;
        match result {
            Ok(devices) => {
                if !devices.is_empty()
                    && app.accounts_tool.devices_selected >= devices.len()
                {
                    app.accounts_tool.devices_selected = devices.len() - 1;
                }
                app.accounts_tool.devices = devices;
                app.accounts_tool.devices_error = None;
            }
            Err(e) => app.accounts_tool.devices_error = Some(e),
        }
    }
}

fn poll_ignored(app: &mut App) {
    let received = app.accounts_tool.ignored_load_rx
        .as_mut()
        .and_then(|rx| rx.try_recv().ok());
    if let Some(result) = received {
        app.accounts_tool.ignored_load_rx = None;
        app.accounts_tool.ignored_loading = false;
        match result {
            Ok(users) => {
                if !users.is_empty()
                    && app.accounts_tool.ignored_selected >= users.len()
                {
                    app.accounts_tool.ignored_selected = users.len() - 1;
                }
                app.accounts_tool.ignored_users = users;
                app.accounts_tool.ignored_error = None;
            }
            Err(e) => app.accounts_tool.ignored_error = Some(e),
        }
    }
}

// ---------------------------------------------------------------------------
// Draw
// ---------------------------------------------------------------------------

const DETAIL_HEIGHT: u16 = 14;

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    if app.accounts_tool.loading {
        f.render_widget(
            Paragraph::new("Loading…")
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let cols = Layout::horizontal([
        Constraint::Percentage(40),
        Constraint::Length(1),
        Constraint::Min(20),
    ])
    .split(area);

    draw_list_panel(f, app, cols[0]);
    draw_right_panel(f, app, cols[2]);

    if app.accounts_tool.filter.active {
        crate::ui::draw_filter_popup(f, &app.accounts_tool.filter, cols[0]);
    }
    if app.accounts_tool.confirm_remove {
        draw_remove_confirm(f, app);
    }
}

fn draw_list_panel(f: &mut Frame, app: &App, area: Rect) {
    if app.accounts_tool.accounts.is_empty() {
        f.render_widget(
            Paragraph::new("No accounts saved.\n\nPress 'a' to add one.")
                .style(Style::default().fg(MUTED))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let filtered = filtered_accounts(app);
    let total = app.accounts_tool.accounts.len();
    let match_count = filtered.len();

    let list_focused = !app.accounts_tool.detail_open;
    let border_color = if list_focused { ACCENT } else { BORDER };
    let title_color = if list_focused { ACCENT } else { MUTED };

    let title = if !app.accounts_tool.filter.input.is_empty() {
        format!(" Accounts ({match_count}/{total}) ")
    } else {
        format!(" Accounts ({total}) ")
    };

    let items: Vec<ListItem> = filtered
        .iter()
        .map(|a| {
            let hs = a.homeserver
                .trim_end_matches('/')
                .trim_start_matches("https://")
                .trim_start_matches("http://");
            let avatar_char = a.user_id
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
                Span::styled(format!("  {hs}"), Style::default().fg(MUTED2)),
                active_badge,
            ]))
        })
        .collect();

    let list = List::new(items)
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
    state.select(Some(app.accounts_tool.selected));

    match &app.accounts_tool.error {
        Some(err) => {
            let chunks = Layout::vertical([
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);
            f.render_stateful_widget(list, chunks[0], &mut state);
            f.render_widget(
                Paragraph::new(err.as_str())
                    .style(Style::default().fg(DANGER))
                    .alignment(Alignment::Center),
                chunks[1],
            );
        }
        None => f.render_stateful_widget(list, area, &mut state),
    }
}

fn draw_right_panel(f: &mut Frame, app: &App, area: Rect) {
    if app.matrix.is_none() {
        f.render_widget(
            Paragraph::new("No active account.\n\nLog in to view details.")
                .style(Style::default().fg(MUTED))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let detail_h = DETAIL_HEIGHT.min(area.height.saturating_sub(5));
    let chunks = Layout::vertical([
        Constraint::Length(detail_h),
        Constraint::Min(3),
    ])
    .split(area);

    draw_detail(f, app, chunks[0]);
    draw_tab_section(f, app, chunks[1]);
}

fn draw_detail(f: &mut Frame, app: &App, area: Rect) {
    let detail_active = app.accounts_tool.detail_open && !app.accounts_tool.detail_tab_focused;
    let border_color = if detail_active { ACCENT } else { BORDER };

    let outer_block = Block::default()
        .title(Span::styled(
            " Account Details ",
            Style::default().fg(border_color),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));
    let inner = outer_block.inner(area);
    f.render_widget(outer_block, area);

    if app.accounts_tool.profile_loading {
        f.render_widget(
            Paragraph::new("Loading…")
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }

    let cx = inner.x + 1;
    let cw = inner.width.saturating_sub(2);
    if cw < 4 || inner.height < 4 { return; }

    let user_id = app.current_user_id.as_deref().unwrap_or("");
    let homeserver = app.matrix.as_ref()
        .map(|c| {
            let hs = c.homeserver_str();
            hs.trim_end_matches('/')
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .to_owned()
        })
        .unwrap_or_default();

    let avatar_letter = user_id
        .trim_start_matches('@')
        .chars()
        .next()
        .unwrap_or('?')
        .to_ascii_uppercase();

    let chunks = Layout::vertical([
        Constraint::Length(1), // [0] padding
        Constraint::Length(1), // [1] avatar + user_id
        Constraint::Length(1), // [2] homeserver
        Constraint::Length(1), // [3] blank
        Constraint::Length(1), // [4] DISPLAY NAME label
        Constraint::Length(1), // [5] display name value
        Constraint::Length(1), // [6] blank
        Constraint::Length(1), // [7] AVATAR URL label
        Constraint::Length(1), // [8] avatar url value
        Constraint::Min(0),    // [9] status/error
    ])
    .split(inner);

    // [1] Avatar + user_id
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {avatar_letter} "),
                Style::default().fg(BG).bg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(user_id.to_owned(), Style::default().fg(FG).add_modifier(Modifier::BOLD)),
        ])),
        Rect::new(cx, chunks[1].y, cw, 1),
    );

    // [2] Homeserver
    f.render_widget(
        Paragraph::new(Span::styled(homeserver, Style::default().fg(MUTED))),
        Rect::new(cx + 5, chunks[2].y, cw.saturating_sub(5), 1),
    );

    // Helper to render a profile field row
    let focused = app.accounts_tool.profile_field;
    let detail_active = app.accounts_tool.detail_open && !app.accounts_tool.detail_tab_focused;

    let render_field = |f: &mut Frame, label: &str, field: ProfileField,
                        edit_val: Option<&str>, stored: Option<&str>, row_y: u16| {
        let is_focused = detail_active && focused == field;
        let is_editing = edit_val.is_some();
        let label_color = if is_focused { ACCENT } else { MUTED };
        let val_color = if is_editing { ACCENT_DIM } else if is_focused { FG } else { FG2 };

        let display_val = if let Some(ev) = edit_val {
            format!("  {ev}█")
        } else {
            format!("  {}", stored.unwrap_or("(not set)"))
        };

        f.render_widget(
            Paragraph::new(Span::styled(label.to_owned(), Style::default().fg(label_color))),
            Rect::new(cx, row_y, cw, 1),
        );
        f.render_widget(
            Paragraph::new(Span::styled(display_val, Style::default().fg(val_color))),
            Rect::new(cx, row_y + 1, cw, 1),
        );
    };

    // [4-5] DISPLAY NAME
    render_field(
        f,
        "DISPLAY NAME",
        ProfileField::DisplayName,
        app.accounts_tool.edit_display_name.as_deref(),
        app.accounts_tool.display_name.as_deref(),
        chunks[4].y,
    );

    // [7-8] AVATAR URL
    render_field(
        f,
        "AVATAR URL",
        ProfileField::AvatarUrl,
        app.accounts_tool.edit_avatar_url.as_deref(),
        app.accounts_tool.avatar_url.as_deref(),
        chunks[7].y,
    );

    // [9] Status/error
    if chunks[9].height > 0 {
        if app.accounts_tool.profile_saving {
            f.render_widget(
                Paragraph::new(Span::styled(
                    "  Saving…",
                    Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC),
                )),
                Rect::new(cx, chunks[9].y, cw, 1),
            );
        } else if let Some(err) = &app.accounts_tool.profile_error {
            f.render_widget(
                Paragraph::new(err.as_str())
                    .style(Style::default().fg(DANGER))
                    .alignment(Alignment::Center),
                Rect::new(inner.x, chunks[9].y, inner.width, 1),
            );
        }
    }
}

fn draw_tab_section(f: &mut Frame, app: &App, area: Rect) {
    let tab_focused = app.accounts_tool.detail_open && app.accounts_tool.detail_tab_focused;
    let border_color = if tab_focused { ACCENT } else { BORDER };

    let devices_active = app.accounts_tool.active_tab == AccountTab::Devices;

    let devices_total = app.accounts_tool.devices.len();
    let devices_matched = filtered_devices(app).len();
    let devices_label = if !app.accounts_tool.devices_filter.input.is_empty() {
        format!("Devices ({devices_matched}/{devices_total})")
    } else {
        format!("Devices ({devices_total})")
    };

    let ignored_total = app.accounts_tool.ignored_users.len();
    let ignored_matched = filtered_ignored(app).len();
    let ignored_label = if !app.accounts_tool.ignored_filter.input.is_empty() {
        format!("Ignored Users ({ignored_matched}/{ignored_total})")
    } else {
        format!("Ignored Users ({ignored_total})")
    };

    let tab_title = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            devices_label,
            Style::default()
                .fg(if devices_active { ACCENT } else { MUTED })
                .add_modifier(if devices_active { Modifier::BOLD } else { Modifier::empty() }),
        ),
        Span::styled(" ─┬─ ", Style::default().fg(border_color)),
        Span::styled(
            ignored_label,
            Style::default()
                .fg(if !devices_active { ACCENT } else { MUTED })
                .add_modifier(if !devices_active { Modifier::BOLD } else { Modifier::empty() }),
        ),
        Span::raw(" "),
    ]);

    let outer_block = Block::default()
        .title(tab_title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .padding(Padding::new(1, 1, 1, 1));
    let inner = outer_block.inner(area);
    f.render_widget(outer_block, area);

    if area.height < 4 { return; }

    match app.accounts_tool.active_tab {
        AccountTab::Devices => draw_devices_list(f, app, inner),
        AccountTab::IgnoredUsers => draw_ignored_list(f, app, inner),
    }

    // Popups on top of the tab section
    if app.accounts_tool.ignored_add_prompt.is_some() {
        draw_ignored_add_prompt(f, app, area);
    }
    if app.accounts_tool.ignored_confirm_unignore {
        draw_unignore_confirm(f, app);
    }
    if app.accounts_tool.delete_dialog.is_some() {
        draw_device_delete_dialog(f, app);
    }

    if app.accounts_tool.devices_filter.active
        && app.accounts_tool.active_tab == AccountTab::Devices
    {
        crate::ui::draw_filter_popup(f, &app.accounts_tool.devices_filter, area);
    }
    if app.accounts_tool.ignored_filter.active
        && app.accounts_tool.active_tab == AccountTab::IgnoredUsers
    {
        crate::ui::draw_filter_popup(f, &app.accounts_tool.ignored_filter, area);
    }
}

fn draw_devices_list(f: &mut Frame, app: &App, area: Rect) {
    if app.accounts_tool.devices_loading {
        f.render_widget(
            Paragraph::new("Loading devices…")
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let filtered = filtered_devices(app);
    let tab_focused = app.accounts_tool.detail_open && app.accounts_tool.detail_tab_focused;

    if filtered.is_empty() {
        let msg = if app.accounts_tool.devices.is_empty() {
            "No devices found. Press 'r' to refresh."
        } else {
            "No matches."
        };
        f.render_widget(
            Paragraph::new(msg).style(Style::default().fg(MUTED)).alignment(Alignment::Center),
            area,
        );
    } else {
        let items: Vec<ListItem> = filtered
            .iter()
            .map(|d| {
                let name = d.display_name.as_deref().unwrap_or("(unnamed)").to_owned();
                let current_marker = if d.is_current {
                    Span::styled(" ✓", Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD))
                } else {
                    Span::raw("")
                };
                let last_info = match (&d.last_seen_ts, &d.last_seen_ip) {
                    (Some(ts), Some(ip)) => format!("  {ts}  {ip}"),
                    (Some(ts), None) => format!("  {ts}"),
                    (None, Some(ip)) => format!("  {ip}"),
                    (None, None) => String::new(),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        name,
                        Style::default()
                            .fg(if d.is_current { SUCCESS } else { FG2 })
                            .add_modifier(if d.is_current { Modifier::BOLD } else { Modifier::empty() }),
                    ),
                    current_marker,
                    Span::styled(format!("  {}", d.device_id), Style::default().fg(MUTED)),
                    Span::styled(last_info, Style::default().fg(MUTED)),
                ]))
            })
            .collect();

        let list = if tab_focused {
            List::new(items)
                .highlight_style(Style::default().bg(BG3).fg(ACCENT_DIM).add_modifier(Modifier::BOLD))
                .highlight_symbol("▌ ")
        } else {
            List::new(items).highlight_symbol("  ")
        };

        let mut state = ListState::default();
        state.select(Some(app.accounts_tool.devices_selected));

        if let Some(err) = &app.accounts_tool.devices_error {
            let chunks = Layout::vertical([
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);
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
    }
}

fn draw_ignored_list(f: &mut Frame, app: &App, area: Rect) {
    if app.accounts_tool.ignored_loading {
        f.render_widget(
            Paragraph::new("Loading ignored users…")
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let filtered = filtered_ignored(app);
    let tab_focused = app.accounts_tool.detail_open && app.accounts_tool.detail_tab_focused;

    // Reserve a row for add prompt and error at bottom
    let has_prompt = app.accounts_tool.ignored_add_prompt.is_some();
    let has_error = app.accounts_tool.ignored_error.is_some();
    let bottom_rows = has_prompt as u16 + has_error as u16;
    let (list_area, bottom_area) = if bottom_rows > 0 && area.height > bottom_rows {
        let c = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(bottom_rows),
        ])
        .split(area);
        (c[0], Some(c[1]))
    } else {
        (area, None)
    };

    if filtered.is_empty() {
        let msg = if app.accounts_tool.ignored_users.is_empty() {
            "No ignored users.\n\nPress 'a' to ignore a user."
        } else {
            "No matches."
        };
        f.render_widget(
            Paragraph::new(msg).style(Style::default().fg(MUTED)).alignment(Alignment::Center),
            list_area,
        );
    } else {
        let items: Vec<ListItem> = filtered
            .iter()
            .map(|u| {
                ListItem::new(Span::styled(
                    (*u).clone(),
                    Style::default().fg(FG2),
                ))
            })
            .collect();

        let list = if tab_focused {
            List::new(items)
                .highlight_style(Style::default().bg(BG3).fg(ACCENT_DIM).add_modifier(Modifier::BOLD))
                .highlight_symbol("▌ ")
        } else {
            List::new(items).highlight_symbol("  ")
        };

        let mut state = ListState::default();
        state.select(Some(app.accounts_tool.ignored_selected));
        f.render_stateful_widget(list, list_area, &mut state);
    }

    if let Some(ba) = bottom_area {
        let mut sub = ba;
        if has_prompt {
            let prompt_text = app.accounts_tool.ignored_add_prompt.as_deref().unwrap_or("");
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(" Ignore user: ", Style::default().fg(ACCENT_DIM)),
                    Span::styled(prompt_text.to_owned(), Style::default().fg(FG)),
                    Span::styled("█", Style::default().fg(ACCENT_DIM)),
                ])),
                Rect::new(sub.x, sub.y, sub.width, 1),
            );
            sub = Rect::new(sub.x, sub.y + 1, sub.width, sub.height.saturating_sub(1));
        }
        if has_error {
            if let Some(err) = &app.accounts_tool.ignored_error {
                f.render_widget(
                    Paragraph::new(err.as_str())
                        .style(Style::default().fg(DANGER))
                        .alignment(Alignment::Center),
                    Rect::new(sub.x, sub.y, sub.width, 1),
                );
            }
        }
    }
}

fn draw_ignored_add_prompt(f: &mut Frame, app: &App, area: Rect) {
    // The add prompt is shown inline in draw_ignored_list, this is a no-op placeholder
    // (the prompt renders in the bottom area of the list, not as a popup).
    let _ = (f, app, area);
}

fn draw_device_delete_dialog(f: &mut Frame, app: &App) {
    let Some((ref device_id, ref dialog_state)) = app.accounts_tool.delete_dialog else {
        return;
    };
    let dev_name = app.accounts_tool.devices.iter()
        .find(|d| &d.device_id == device_id)
        .and_then(|d| d.display_name.clone())
        .unwrap_or_else(|| device_id.clone());

    let area = f.area();
    let popup = centered_rect(58, 9, area);
    f.render_widget(Clear, popup);

    let lines = match dialog_state {
        DeviceDeleteDialog::Confirm => vec![
            Line::from(""),
            Line::from(vec![
                Span::raw("  Sign out device: "),
                Span::styled(dev_name, Style::default().fg(ACCENT_DIM).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  y/Enter", Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)),
                Span::raw("  continue    "),
                Span::styled("any other key", Style::default().fg(DANGER)),
                Span::raw("  cancel"),
            ]),
        ],
        DeviceDeleteDialog::EnterPassword(pwd) => vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Enter your account password to confirm:",
                Style::default().fg(FG),
            )]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Password: ", Style::default().fg(ACCENT_DIM)),
                Span::styled("•".repeat(pwd.len()), Style::default().fg(FG)),
                Span::styled("█", Style::default().fg(ACCENT_DIM)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Enter", Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)),
                Span::raw("  confirm    "),
                Span::styled("Esc", Style::default().fg(ACCENT)),
                Span::raw("  back"),
            ]),
        ],
    };

    f.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(Span::styled(
                        " Sign Out Device ",
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

fn draw_unignore_confirm(f: &mut Frame, app: &App) {
    let users = filtered_ignored(app);
    let subject = format!(
        "Unignore {}?",
        users.get(app.accounts_tool.ignored_selected).map(|s| s.as_str()).unwrap_or("this user")
    );
    draw_confirm_popup(f, "Confirm", &subject);
}

fn draw_remove_confirm(f: &mut Frame, app: &App) {
    let accounts = filtered_accounts(app);
    let subject = format!(
        "Remove account {}?",
        accounts.get(app.accounts_tool.selected).map(|a| a.user_id.as_str()).unwrap_or("this account")
    );
    draw_confirm_popup(f, "Remove Account", &subject);
}

// ---------------------------------------------------------------------------
// Hint spans
// ---------------------------------------------------------------------------

pub fn hint_spans(app: &App) -> Vec<Span<'static>> {
    if app.accounts_tool.delete_dialog.is_some() {
        return vec![];
    }
    if app.accounts_tool.confirm_remove {
        return vec![];
    }
    if app.accounts_tool.ignored_confirm_unignore {
        return vec![];
    }
    if app.accounts_tool.ignored_add_prompt.is_some() {
        return hint_spans_from_cmds(CMDS_IGNORED_ADD);
    }
    if app.accounts_tool.detail_open && app.accounts_tool.detail_tab_focused {
        match app.accounts_tool.active_tab {
            AccountTab::Devices => {
                if app.accounts_tool.devices_filter.active {
                    return filter_hint_spans(
                        app.accounts_tool.devices_filter.column,
                        DeviceInfo::filter_cols(),
                    );
                }
                return hint_spans_from_cmds(CMDS_DEVICES);
            }
            AccountTab::IgnoredUsers => {
                if app.accounts_tool.ignored_filter.active {
                    return vec![
                        Span::styled("type", Style::default().fg(ACCENT)),
                        Span::raw(" to filter  "),
                        Span::styled("Enter", Style::default().fg(ACCENT)),
                        Span::raw(" close  "),
                        Span::styled("Esc", Style::default().fg(ACCENT)),
                        Span::raw(" clear"),
                    ];
                }
                return hint_spans_from_cmds(CMDS_IGNORED);
            }
        }
    }
    if app.accounts_tool.detail_open {
        if app.accounts_tool.is_profile_editing() {
            return hint_spans_from_cmds(CMDS_EDITING);
        }
        return hint_spans_from_cmds(CMDS_DETAIL);
    }
    if app.accounts_tool.filter.active {
        return filter_hint_spans(
            app.accounts_tool.filter.column,
            AccountSummary::filter_cols(),
        );
    }
    hint_spans_from_cmds(CMDS_LIST)
}
