use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::Backend;

use crate::matrix::MatrixClient;
use crate::tools::{accounts, devices, home, ignore_list, profile, rooms};
use crate::ui;

// ---------------------------------------------------------------------------
// Commands (k9s-style)
// ---------------------------------------------------------------------------

pub const COMMANDS: &[(&str, &str)] = &[
    ("home", "Tool selection screen"),
    ("rooms", "Browse and manage rooms"),
    ("accounts", "Manage accounts"),
    ("ignorelist", "Manage ignored users"),
    ("profile", "Edit display name and avatar"),
    ("devices", "Manage logged-in devices"),
    ("help", "Keyboard shortcut reference"),
    ("login", "Add a new account"),
    ("quit", "Quit"),
];

pub const HOME_TOOLS: &[(&str, &str)] = &[
    ("Rooms", "rooms"),
    ("Accounts", "accounts"),
    ("Ignore List", "ignorelist"),
    ("Profile", "profile"),
    ("Devices", "devices"),
];

// ---------------------------------------------------------------------------
// Login form
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoginField {
    #[default]
    Homeserver,
    Username,
    Password,
}

impl LoginField {
    pub fn next(self) -> Self {
        match self {
            Self::Homeserver => Self::Username,
            Self::Username => Self::Password,
            Self::Password => Self::Homeserver,
        }
    }
    pub fn prev(self) -> Self {
        match self {
            Self::Homeserver => Self::Password,
            Self::Username => Self::Homeserver,
            Self::Password => Self::Username,
        }
    }
}

#[derive(Debug, Default)]
pub struct LoginState {
    pub homeserver: String,
    pub username: String,
    pub password: String,
    pub focused: LoginField,
    pub error: Option<String>,
    pub loading: bool,
    pub can_go_back: bool,
}

// ---------------------------------------------------------------------------
// Top-level screen / tool
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum Screen {
    Login,
    Main,
    Quitting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTool {
    Home,
    Rooms,
    Accounts,
    IgnoreList,
    Profile,
    Devices,
}

// ---------------------------------------------------------------------------
// Command bar
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct CommandBarState {
    pub input: String,
}

impl CommandBarState {
    pub fn completions(&self) -> Vec<&'static str> {
        let lower = self.input.to_lowercase();
        COMMANDS
            .iter()
            .filter(|(cmd, _)| cmd.starts_with(lower.as_str()))
            .map(|(cmd, _)| *cmd)
            .collect()
    }

    pub fn best_completion(&self) -> Option<&'static str> {
        self.completions().into_iter().next()
    }
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub struct App {
    pub screen: Screen,
    pub active_tool: ActiveTool,
    pub command_bar: Option<CommandBarState>,
    pub show_help: bool,

    // Per-screen / tool state
    pub login: LoginState,
    pub home: home::HomeState,
    pub accounts_tool: accounts::AccountsToolState,
    pub rooms_tool: rooms::RoomBrowserState,
    pub ignore_list: ignore_list::IgnoreListState,
    pub profile: profile::ProfileState,
    pub devices: devices::DevicesState,

    // Matrix
    pub matrix: Option<MatrixClient>,
    pub current_user_id: Option<String>,
    pub sync_task: Option<tokio::task::JoinHandle<()>>,
}

impl App {
    /// Build the app — fast, no network calls.
    pub async fn new() -> Self {
        let mut app = App {
            screen: Screen::Login,
            active_tool: ActiveTool::Home,
            command_bar: None,
            show_help: false,
            login: LoginState::default(),
            home: home::HomeState::default(),
            accounts_tool: accounts::AccountsToolState::default(),
            rooms_tool: rooms::RoomBrowserState::default(),
            ignore_list: ignore_list::IgnoreListState::default(),
            profile: profile::ProfileState::default(),
            devices: devices::DevicesState::default(),
            matrix: None,
            current_user_id: None,
            sync_task: None,
        };

        match MatrixClient::restore_current().await {
            Ok(Some(client)) => {
                app.sync_task = Some(client.start_background_sync());
                app.current_user_id = Some(client.user_id());
                app.matrix = Some(client);
                app.screen = Screen::Main;
            }
            Ok(None) => {}
            Err(e) => {
                app.login.error =
                    Some(format!("Session restore failed: {e}. Please log in again."));
            }
        }

        app
    }

    pub async fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()>
    where
        B::Error: Send + Sync + 'static,
    {
        // Draw immediately — before any network call.
        terminal.draw(|f| ui::draw(f, self))?;

        loop {
            rooms::poll_leave_results(self);
            rooms::poll_member_load(self);
            ignore_list::poll_load(self);
            profile::poll_load(self);
            devices::poll_load(self);

            terminal.draw(|f| ui::draw(f, self))?;

            if matches!(self.screen, Screen::Quitting) {
                break;
            }

            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        handle_key(self, key.code, key.modifiers).await;
                    }
                }
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Input dispatch
// ---------------------------------------------------------------------------

async fn handle_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
        app.screen = Screen::Quitting;
        return;
    }

    if app.command_bar.is_some() {
        handle_command_bar_key(app, code).await;
        return;
    }

    match app.screen {
        Screen::Login => handle_login_key(app, code).await,
        Screen::Main => handle_main_key(app, code).await,
        Screen::Quitting => {}
    }
}

async fn handle_command_bar_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.command_bar = None;
        }
        KeyCode::Enter => {
            if let Some(bar) = &app.command_bar {
                let cmd = bar
                    .best_completion()
                    .map(|s| s.to_owned())
                    .unwrap_or_else(|| bar.input.clone());
                app.command_bar = None;
                execute_command(app, &cmd).await;
            }
        }
        KeyCode::Tab => {
            if let Some(bar) = &mut app.command_bar {
                if let Some(c) = bar.best_completion() {
                    bar.input = c.to_owned();
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(bar) = &mut app.command_bar {
                bar.input.pop();
            }
        }
        KeyCode::Char(c) => {
            if let Some(bar) = &mut app.command_bar {
                bar.input.push(c);
            }
        }
        _ => {}
    }
}

pub async fn execute_command(app: &mut App, cmd: &str) {
    match cmd.trim() {
        "home" | "h" => app.active_tool = ActiveTool::Home,
        "rooms" => navigate_to_rooms(app).await,
        "accounts" | "a" => navigate_to_accounts(app).await,
        "ignorelist" => navigate_to_ignore_list(app),
        "profile" => navigate_to_profile(app),
        "devices" => navigate_to_devices(app),
        "help" => app.show_help = true,
        "login" | "l" => {
            app.login = LoginState {
                can_go_back: true,
                ..LoginState::default()
            };
            app.screen = Screen::Login;
        }
        "quit" | "q" => app.screen = Screen::Quitting,
        _ => {}
    }
}

// Navigate helpers

async fn navigate_to_rooms(app: &mut App) {
    app.active_tool = ActiveTool::Rooms;
    if app.rooms_tool.rooms.is_empty() && !app.rooms_tool.loading {
        app.rooms_tool.loading = true;
        rooms::do_load_rooms(app).await;
    }
}

async fn navigate_to_accounts(app: &mut App) {
    app.active_tool = ActiveTool::Accounts;
    app.accounts_tool.loading = true;
    accounts::do_load_accounts(app).await;
}

fn navigate_to_ignore_list(app: &mut App) {
    app.active_tool = ActiveTool::IgnoreList;
    if !app.ignore_list.loading {
        ignore_list::start_load(app);
    }
}

fn navigate_to_profile(app: &mut App) {
    app.active_tool = ActiveTool::Profile;
    if !app.profile.loading {
        profile::start_load(app);
    }
}

fn navigate_to_devices(app: &mut App) {
    app.active_tool = ActiveTool::Devices;
    if !app.devices.loading {
        devices::start_load(app);
    }
}

// ---------------------------------------------------------------------------
// Login screen
// ---------------------------------------------------------------------------

async fn handle_login_key(app: &mut App, code: KeyCode) {
    if app.login.loading {
        return;
    }
    match code {
        KeyCode::Tab => app.login.focused = app.login.focused.next(),
        KeyCode::BackTab => app.login.focused = app.login.focused.prev(),
        KeyCode::Up => app.login.focused = app.login.focused.prev(),
        KeyCode::Down => app.login.focused = app.login.focused.next(),
        KeyCode::Esc if app.login.can_go_back => {
            app.screen = Screen::Main;
        }
        KeyCode::Enter => {
            if app.login.focused != LoginField::Password {
                app.login.focused = app.login.focused.next();
            } else {
                do_login(app).await;
            }
        }
        KeyCode::Backspace => {
            focused_login_field_mut(app).pop();
        }
        KeyCode::Char(c) => {
            focused_login_field_mut(app).push(c);
        }
        _ => {}
    }
}

fn focused_login_field_mut(app: &mut App) -> &mut String {
    match app.login.focused {
        LoginField::Homeserver => &mut app.login.homeserver,
        LoginField::Username => &mut app.login.username,
        LoginField::Password => &mut app.login.password,
    }
}

async fn do_login(app: &mut App) {
    app.login.error = None;
    app.login.loading = true;

    let homeserver = app.login.homeserver.trim().to_owned();
    let username = app.login.username.trim().to_owned();
    let password = app.login.password.clone();

    match MatrixClient::login(&homeserver, &username, &password).await {
        Ok(client) => {
            if let Some(task) = app.sync_task.take() {
                task.abort();
            }
            app.sync_task = Some(client.start_background_sync());
            app.current_user_id = Some(client.user_id());
            app.matrix = Some(client);
            app.login.loading = false;
            app.screen = Screen::Main;
            app.active_tool = ActiveTool::Home;
            accounts::do_load_accounts(app).await;
        }
        Err(e) => {
            app.login.loading = false;
            app.login.error = Some(format!("{e}"));
        }
    }
}

// ---------------------------------------------------------------------------
// Main screen dispatch
// ---------------------------------------------------------------------------

async fn handle_main_key(app: &mut App, code: KeyCode) {
    // Help overlay intercepts Esc and ?.
    if app.show_help {
        if matches!(code, KeyCode::Esc | KeyCode::Char('?')) {
            app.show_help = false;
        }
        return;
    }

    if code == KeyCode::Char(':') && !is_text_input_active(app) {
        app.command_bar = Some(CommandBarState::default());
        return;
    }
    if code == KeyCode::Char('?') {
        app.show_help = true;
        return;
    }

    match app.active_tool {
        ActiveTool::Home => home::handle(app, code).await,
        ActiveTool::Rooms => rooms::handle(app, code).await,
        ActiveTool::Accounts => accounts::handle(app, code).await,
        ActiveTool::IgnoreList => ignore_list::handle(app, code).await,
        ActiveTool::Profile => profile::handle(app, code).await,
        ActiveTool::Devices => devices::handle(app, code).await,
    }
}

fn is_text_input_active(app: &App) -> bool {
    use crate::tools::devices::DeleteDialogState;
    app.rooms_tool.detail.editing.is_some()
        || app.rooms_tool.members.as_ref().map_or(false, |m| m.pl_edit.is_some())
        || app.ignore_list.add_prompt.is_some()
        || app.profile.is_editing()
        || matches!(
            app.devices.delete_dialog,
            Some((_, DeleteDialogState::EnterPassword(_)))
        )
}
