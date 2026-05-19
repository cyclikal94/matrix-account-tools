use std::collections::HashSet;

use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use tokio::sync::{mpsc, oneshot};

use crate::app::{ActiveTool, App};
use crate::matrix::{MemberInfo, RoomInfo};
use crate::tools::{ACCENT, ACCENT_DIM, BG, BG2, BG3, BORDER, DANGER, FG2, MUTED, MUTED2, SUCCESS, FilterState};
use crate::ui::centered_rect;

// ---------------------------------------------------------------------------
// Leave item (parallel leaving progress)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaveStatus {
    InProgress,
    Done,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct LeaveItem {
    pub room_id: String,
    pub room_name: String,
    pub status: LeaveStatus,
}

// ---------------------------------------------------------------------------
// Detail view sub-state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailField {
    Name,
    Topic,
    Alias,
}

impl Default for DetailField {
    fn default() -> Self {
        Self::Name
    }
}

#[derive(Debug, Default)]
pub struct DetailState {
    pub focused: DetailField,
    pub editing: Option<String>,
    pub saving: bool,
    pub error: Option<String>,
    pub success: Option<String>,
    pub confirm_leave: bool,
}

// ---------------------------------------------------------------------------
// Member view sub-state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModAction {
    Kick,
    Ban,
}

#[derive(Debug, Default)]
pub struct MembersState {
    pub members: Vec<MemberInfo>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub confirm: Option<(ModAction, String)>,
    pub pl_edit: Option<String>, // Some(input) when editing a member's power level
    pub load_rx: Option<oneshot::Receiver<Result<Vec<MemberInfo>, String>>>,
    pub action_rx: Option<oneshot::Receiver<Result<(), String>>>,
}

// ---------------------------------------------------------------------------
// Top-level state
// ---------------------------------------------------------------------------

pub struct RoomBrowserState {
    pub rooms: Vec<RoomInfo>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub filter: FilterState,

    // Leave-select mode (list view)
    pub leave_select: bool,
    pub checked: HashSet<String>,
    pub leaving_items: Vec<LeaveItem>,
    pub leave_rx: Option<mpsc::Receiver<(String, Result<(), String>)>>,

    // Detail view state (always visible in right panel)
    pub detail: DetailState,

    // Member view (Some = member list open)
    pub members: Option<MembersState>,

    // Whether the detail panel is "open" (user pressed Enter on a room).
    pub detail_open: bool,
}

impl Default for RoomBrowserState {
    fn default() -> Self {
        Self {
            rooms: Vec::new(),
            selected: 0,
            loading: false,
            error: None,
            filter: FilterState::default(),
            leave_select: false,
            checked: HashSet::new(),
            leaving_items: Vec::new(),
            leave_rx: None,
            detail: DetailState::default(),
            members: None,
            detail_open: false,
        }
    }
}

impl RoomBrowserState {
    pub fn filtered_rooms(&self) -> Vec<&RoomInfo> {
        self.rooms
            .iter()
            .filter(|r| self.filter.matches(&r.display_name))
            .collect()
    }

    pub fn selected_room_id(&self) -> Option<String> {
        self.filtered_rooms()
            .get(self.selected)
            .map(|r| r.id.clone())
    }

    pub fn selected_room_idx(&self) -> Option<usize> {
        let id = self.selected_room_id()?;
        self.rooms.iter().position(|r| r.id == id)
    }
}

// ---------------------------------------------------------------------------
// Poll (called from run loop)
// ---------------------------------------------------------------------------

pub fn poll_leave_results(app: &mut App) {
    loop {
        let msg = match app.rooms_tool.leave_rx.as_mut() {
            Some(rx) => match rx.try_recv() {
                Ok(msg) => msg,
                Err(_) => break,
            },
            None => break,
        };
        let (room_id, result) = msg;
        if let Some(item) = app
            .rooms_tool
            .leaving_items
            .iter_mut()
            .find(|i| i.room_id == room_id)
        {
            item.status = match result {
                Ok(()) => LeaveStatus::Done,
                Err(e) => LeaveStatus::Failed(e),
            };
        }
    }

    // Once all are settled, clear after a short display window.
    if !app.rooms_tool.leaving_items.is_empty()
        && app
            .rooms_tool
            .leaving_items
            .iter()
            .all(|i| matches!(i.status, LeaveStatus::Done | LeaveStatus::Failed(_)))
    {
        // Keep channel drained; drop it to signal completion.
        app.rooms_tool.leave_rx = None;
        // Reload rooms to reflect leaves.
        let rooms = app.rooms_tool.leaving_items.clone();
        for item in &rooms {
            app.rooms_tool.rooms.retain(|r| r.id != item.room_id || matches!(item.status, LeaveStatus::Failed(_)));
        }
        app.rooms_tool.checked.clear();
        app.rooms_tool.leave_select = false;
        // leave leaving_items visible briefly — user can press Esc/q to clear
    }
}

pub fn poll_member_load(app: &mut App) {
    let received = app
        .rooms_tool
        .members
        .as_mut()
        .and_then(|m| m.load_rx.as_mut())
        .and_then(|rx| rx.try_recv().ok());

    if let Some(result) = received {
        if let Some(ms) = &mut app.rooms_tool.members {
            ms.load_rx = None;
            ms.loading = false;
            match result {
                Ok(members) => {
                    ms.members = members;
                    ms.error = None;
                }
                Err(e) => {
                    ms.error = Some(e);
                }
            }
        }
    }

    let action_done = app
        .rooms_tool
        .members
        .as_mut()
        .and_then(|m| m.action_rx.as_mut())
        .and_then(|rx| rx.try_recv().ok());

    if let Some(result) = action_done {
        if let Some(ms) = &mut app.rooms_tool.members {
            ms.action_rx = None;
            match result {
                Ok(()) => start_member_load(app),
                Err(e) => {
                    if let Some(ms) = &mut app.rooms_tool.members {
                        ms.error = Some(e);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn handle(app: &mut App, code: KeyCode) {
    // Member view.
    if app.rooms_tool.members.is_some() {
        handle_members(app, code).await;
        return;
    }

    // Detail edit/confirm-leave takes priority when active.
    if app.rooms_tool.detail.editing.is_some() || app.rooms_tool.detail.confirm_leave {
        handle_detail_edit(app, code).await;
        return;
    }

    // Leaving progress overlay — any key clears if done.
    if !app.rooms_tool.leaving_items.is_empty() {
        let all_done = app
            .rooms_tool
            .leaving_items
            .iter()
            .all(|i| matches!(i.status, LeaveStatus::Done | LeaveStatus::Failed(_)));
        if all_done && matches!(code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter) {
            app.rooms_tool.leaving_items.clear();
        }
        return;
    }

    // Filter input.
    if app.rooms_tool.filter.active {
        handle_filter_input(app, code);
        return;
    }

    // Leave-select mode.
    if app.rooms_tool.leave_select {
        handle_leave_select(app, code).await;
        return;
    }

    // When "inside" a room, dispatch to detail-view handler.
    if app.rooms_tool.detail_open {
        handle_detail_view(app, code).await;
        return;
    }

    // Normal list navigation.
    handle_list(app, code).await;
}

fn handle_filter_input(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => app.rooms_tool.filter.clear(),
        KeyCode::Backspace => {
            app.rooms_tool.filter.input.pop();
        }
        KeyCode::Char(c) if !c.is_control() => {
            app.rooms_tool.filter.input.push(c);
            app.rooms_tool.selected = 0;
        }
        KeyCode::Down | KeyCode::Char('j') => nav_down(app),
        KeyCode::Up | KeyCode::Char('k') => nav_up(app),
        _ => {}
    }
}

async fn handle_list(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            app.active_tool = ActiveTool::Home;
            app.rooms_tool.detail_open = false;
            app.rooms_tool.detail = DetailState::default();
        }
        KeyCode::Char('j') | KeyCode::Down => nav_down(app),
        KeyCode::Char('k') | KeyCode::Up => nav_up(app),
        KeyCode::Char('/') => {
            app.rooms_tool.filter.active = true;
            app.rooms_tool.filter.input.clear();
            app.rooms_tool.selected = 0;
        }
        KeyCode::Enter => {
            if !app.rooms_tool.filtered_rooms().is_empty() {
                app.rooms_tool.detail_open = true;
            }
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            app.rooms_tool.leave_select = true;
            app.rooms_tool.checked.clear();
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.rooms_tool.loading = true;
            do_load_rooms(app).await;
        }
        _ => {}
    }
}

async fn handle_detail_view(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.rooms_tool.detail_open = false;
        }
        KeyCode::Char('j') | KeyCode::Down => nav_down(app),
        KeyCode::Char('k') | KeyCode::Up => nav_up(app),
        KeyCode::Char('x') | KeyCode::Char('X') => {
            if !app.rooms_tool.rooms.is_empty() {
                app.rooms_tool.detail.confirm_leave = true;
            }
        }
        KeyCode::Char('m') | KeyCode::Char('M') => {
            app.rooms_tool.members = Some(MembersState::default());
            start_member_load(app);
        }
        KeyCode::Char('e') | KeyCode::Char('E') => {
            if let Some(idx) = app.rooms_tool.selected_room_idx() {
                if let Some(room) = app.rooms_tool.rooms.get(idx) {
                    let current = match app.rooms_tool.detail.focused {
                        DetailField::Name => room.display_name.clone(),
                        DetailField::Topic => room.topic.clone().unwrap_or_default(),
                        DetailField::Alias => room.alias.clone().unwrap_or_default(),
                    };
                    app.rooms_tool.detail.editing = Some(current);
                    app.rooms_tool.detail.error = None;
                    app.rooms_tool.detail.success = None;
                }
            }
        }
        KeyCode::Tab => {
            app.rooms_tool.detail.focused = match app.rooms_tool.detail.focused {
                DetailField::Name => DetailField::Topic,
                DetailField::Topic => DetailField::Alias,
                DetailField::Alias => DetailField::Name,
            };
        }
        _ => {}
    }
}

async fn handle_leave_select(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.rooms_tool.leave_select = false;
            app.rooms_tool.checked.clear();
        }
        KeyCode::Char('j') | KeyCode::Down => nav_down(app),
        KeyCode::Char('k') | KeyCode::Up => nav_up(app),
        KeyCode::Char(' ') => {
            let filtered = app.rooms_tool.filtered_rooms();
            if let Some(room) = filtered.get(app.rooms_tool.selected) {
                let id = room.id.clone();
                if app.rooms_tool.checked.contains(&id) {
                    app.rooms_tool.checked.remove(&id);
                } else {
                    app.rooms_tool.checked.insert(id);
                }
            }
        }
        KeyCode::Enter => start_leaving(app),
        _ => {}
    }
}

/// Handles input when detail.editing is Some or detail.confirm_leave is true.
async fn handle_detail_edit(app: &mut App, code: KeyCode) {
    if app.rooms_tool.detail.confirm_leave {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                do_leave_single(app).await;
            }
            _ => app.rooms_tool.detail.confirm_leave = false,
        }
        return;
    }

    if app.rooms_tool.detail.saving {
        return;
    }

    if let Some(ref val) = app.rooms_tool.detail.editing.clone() {
        match code {
            KeyCode::Esc => app.rooms_tool.detail.editing = None,
            KeyCode::Backspace => {
                let mut s = val.clone();
                s.pop();
                app.rooms_tool.detail.editing = Some(s);
            }
            KeyCode::Char(c) if !c.is_control() => {
                let mut s = val.clone();
                s.push(c);
                app.rooms_tool.detail.editing = Some(s);
            }
            KeyCode::Enter => do_save_field(app).await,
            _ => {}
        }
    }
}

async fn handle_members(app: &mut App, code: KeyCode) {
    let Some(ms) = &app.rooms_tool.members else { return; };

    // Power level input mode.
    if let Some(ref input) = ms.pl_edit.clone() {
        match code {
            KeyCode::Esc => {
                app.rooms_tool.members.as_mut().unwrap().pl_edit = None;
            }
            KeyCode::Backspace => {
                let mut s = input.clone();
                s.pop();
                app.rooms_tool.members.as_mut().unwrap().pl_edit = Some(s);
            }
            KeyCode::Char(c) if c.is_ascii_digit() || (c == '-' && input.is_empty()) => {
                let mut s = input.clone();
                s.push(c);
                app.rooms_tool.members.as_mut().unwrap().pl_edit = Some(s);
            }
            KeyCode::Enter => {
                let input = input.clone();
                let ms = app.rooms_tool.members.as_ref().unwrap();
                let user_id = ms.members.get(ms.selected).map(|m| m.user_id.clone());
                if let (Some(uid), Ok(level)) = (user_id, input.parse::<i64>()) {
                    app.rooms_tool.members.as_mut().unwrap().pl_edit = None;
                    do_set_power_level(app, uid, level).await;
                } else {
                    app.rooms_tool.members.as_mut().unwrap().error =
                        Some("Invalid power level — enter an integer.".to_owned());
                    app.rooms_tool.members.as_mut().unwrap().pl_edit = None;
                }
            }
            _ => {}
        }
        return;
    }

    // Confirm dialog.
    if let Some((action, user_id)) = ms.confirm.clone() {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                app.rooms_tool.members.as_mut().unwrap().confirm = None;
                do_mod_action(app, action, user_id).await;
            }
            _ => {
                app.rooms_tool.members.as_mut().unwrap().confirm = None;
            }
        }
        return;
    }

    if ms.loading { return; }

    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.rooms_tool.members = None;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let ms = app.rooms_tool.members.as_mut().unwrap();
            if ms.selected + 1 < ms.members.len() {
                ms.selected += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let ms = app.rooms_tool.members.as_mut().unwrap();
            if ms.selected > 0 {
                ms.selected -= 1;
            }
        }
        KeyCode::Char('p') | KeyCode::Char('P') => {
            let ms = app.rooms_tool.members.as_ref().unwrap();
            if let Some(m) = ms.members.get(ms.selected) {
                if m.can_set_power_level {
                    let current = m.power_level.to_string();
                    app.rooms_tool.members.as_mut().unwrap().pl_edit = Some(current);
                    app.rooms_tool.members.as_mut().unwrap().error = None;
                } else if m.is_self {
                    app.rooms_tool.members.as_mut().unwrap().error =
                        Some("Cannot change your own power level.".to_owned());
                } else {
                    app.rooms_tool.members.as_mut().unwrap().error =
                        Some("Insufficient permissions.".to_owned());
                }
            }
        }
        KeyCode::Char('K') => {
            let ms = app.rooms_tool.members.as_ref().unwrap();
            if let Some(m) = ms.members.get(ms.selected) {
                if m.can_kick {
                    let uid = m.user_id.clone();
                    app.rooms_tool.members.as_mut().unwrap().confirm = Some((ModAction::Kick, uid));
                } else {
                    app.rooms_tool.members.as_mut().unwrap().error =
                        Some("Insufficient permissions to kick.".to_owned());
                }
            }
        }
        KeyCode::Char('b') | KeyCode::Char('B') => {
            let ms = app.rooms_tool.members.as_ref().unwrap();
            if let Some(m) = ms.members.get(ms.selected) {
                if m.can_ban {
                    let uid = m.user_id.clone();
                    app.rooms_tool.members.as_mut().unwrap().confirm =
                        Some((ModAction::Ban, uid));
                } else {
                    app.rooms_tool.members.as_mut().unwrap().error =
                        Some("Insufficient permissions to ban.".to_owned());
                }
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') => start_member_load(app),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

fn nav_down(app: &mut App) {
    let len = app.rooms_tool.filtered_rooms().len();
    if app.rooms_tool.selected + 1 < len {
        app.rooms_tool.selected += 1;
    }
}

fn nav_up(app: &mut App) {
    if app.rooms_tool.selected > 0 {
        app.rooms_tool.selected -= 1;
    }
}

fn start_leaving(app: &mut App) {
    let to_leave: Vec<(String, String)> = app
        .rooms_tool
        .filtered_rooms()
        .iter()
        .filter(|r| app.rooms_tool.checked.contains(&r.id))
        .map(|r| (r.id.clone(), r.display_name.clone()))
        .collect();

    if to_leave.is_empty() {
        return;
    }

    let (tx, rx) = mpsc::channel(to_leave.len().max(1));
    app.rooms_tool.leave_rx = Some(rx);
    app.rooms_tool.leaving_items = to_leave
        .iter()
        .map(|(id, name)| LeaveItem {
            room_id: id.clone(),
            room_name: name.clone(),
            status: LeaveStatus::InProgress,
        })
        .collect();
    app.rooms_tool.leave_select = false;

    for (room_id, _) in to_leave {
        if let Some(client) = app.matrix.clone() {
            let tx = tx.clone();
            tokio::spawn(async move {
                let result = client
                    .leave_room(&room_id)
                    .await
                    .map_err(|e| e.to_string());
                let _ = tx.send((room_id, result)).await;
            });
        }
    }
}

async fn do_leave_single(app: &mut App) {
    app.rooms_tool.detail.confirm_leave = false;
    let Some(room_id) = app.rooms_tool.selected_room_id() else { return; };

    if let Some(client) = &app.matrix {
        match client.leave_room(&room_id).await {
            Ok(()) => {
                app.rooms_tool.rooms.retain(|r| r.id != room_id);
                // Clamp selected
                let filtered_len = app.rooms_tool.filtered_rooms().len();
                if app.rooms_tool.selected >= filtered_len && filtered_len > 0 {
                    app.rooms_tool.selected = filtered_len - 1;
                }
                app.rooms_tool.detail = DetailState::default();
                app.rooms_tool.members = None;
            }
            Err(e) => {
                app.rooms_tool.detail.error = Some(format!("Leave failed: {e}"));
            }
        }
    }
}

async fn do_save_field(app: &mut App) {
    let Some(idx) = app.rooms_tool.selected_room_idx() else { return; };
    let Some(room) = app.rooms_tool.rooms.get(idx) else { return; };
    let room_id = room.id.clone();
    let val = app.rooms_tool.detail.editing.take().unwrap_or_default();

    app.rooms_tool.detail.saving = true;
    app.rooms_tool.detail.error = None;

    let result = if let Some(client) = &app.matrix {
        match app.rooms_tool.detail.focused {
            DetailField::Name => client.set_room_name(&room_id, val.clone()).await,
            DetailField::Topic => client.set_room_topic(&room_id, &val).await,
            DetailField::Alias => {
                let a = if val.is_empty() { None } else { Some(val.as_str()) };
                client.set_room_canonical_alias(&room_id, a).await
            }
        }
    } else {
        Err(anyhow::anyhow!("Not connected"))
    };

    match result {
        Ok(()) => {
            if let Some(room) = app.rooms_tool.rooms.get_mut(idx) {
                match app.rooms_tool.detail.focused {
                    DetailField::Name => room.display_name = val,
                    DetailField::Topic => room.topic = if val.is_empty() { None } else { Some(val) },
                    DetailField::Alias => room.alias = if val.is_empty() { None } else { Some(val) },
                }
            }
            app.rooms_tool.detail.success = Some("Saved!".to_owned());
        }
        Err(e) => {
            app.rooms_tool.detail.error = Some(format!("{e}"));
        }
    }
    app.rooms_tool.detail.saving = false;
}

fn start_member_load(app: &mut App) {
    let Some(room_id) = app.rooms_tool.selected_room_id() else { return; };
    let Some(client) = app.matrix.clone() else { return; };
    let ms = match &mut app.rooms_tool.members {
        Some(ms) => ms,
        None => return,
    };
    ms.loading = true;
    ms.error = None;
    let (tx, rx) = oneshot::channel();
    ms.load_rx = Some(rx);
    tokio::spawn(async move {
        let result = client.get_room_members(&room_id).await.map_err(|e| e.to_string());
        let _ = tx.send(result);
    });
}

async fn do_set_power_level(app: &mut App, user_id: String, level: i64) {
    let Some(room_id) = app.rooms_tool.selected_room_id() else { return; };
    let Some(client) = app.matrix.clone() else { return; };

    let (tx, rx) = oneshot::channel();
    if let Some(ms) = &mut app.rooms_tool.members {
        ms.action_rx = Some(rx);
        ms.loading = true;
    }
    tokio::spawn(async move {
        let result = client.set_member_power_level(&room_id, &user_id, level).await;
        let _ = tx.send(result.map_err(|e| e.to_string()));
    });
}

async fn do_mod_action(app: &mut App, action: ModAction, user_id: String) {
    let Some(room_id) = app.rooms_tool.selected_room_id() else { return; };
    let Some(client) = app.matrix.clone() else { return; };

    let (tx, rx) = oneshot::channel();
    if let Some(ms) = &mut app.rooms_tool.members {
        ms.action_rx = Some(rx);
        ms.loading = true;
    }
    tokio::spawn(async move {
        let result = match action {
            ModAction::Kick => client.kick_member(&room_id, &user_id).await,
            ModAction::Ban => client.ban_member(&room_id, &user_id).await,
        };
        let _ = tx.send(result.map_err(|e| e.to_string()));
    });
}

pub async fn do_load_rooms(app: &mut App) {
    if let Some(client) = &app.matrix {
        match client.get_joined_rooms().await {
            Ok(rooms) => {
                app.rooms_tool.rooms = rooms;
                app.rooms_tool.error = None;
                app.rooms_tool.selected = 0;
            }
            Err(e) => {
                app.rooms_tool.error = Some(format!("{e}"));
            }
        }
    }
    app.rooms_tool.loading = false;
}

// ---------------------------------------------------------------------------
// Draw
// ---------------------------------------------------------------------------

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    if app.rooms_tool.loading {
        f.render_widget(
            Paragraph::new("Syncing rooms…")
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    // 56% list, 1 separator, rest for detail — matches design proportions.
    let [left, sep, right] = Layout::horizontal([
        Constraint::Percentage(56),
        Constraint::Length(1),
        Constraint::Min(10),
    ])
    .areas(area);

    // Vertical separator line.
    for row in sep.y..sep.y + sep.height {
        f.render_widget(
            Paragraph::new("│").style(Style::default().fg(BORDER)),
            Rect::new(sep.x, row, 1, 1),
        );
    }

    draw_list_panel(f, app, left);
    draw_right_panel(f, app, right);
}

fn draw_right_panel(f: &mut Frame, app: &App, area: Rect) {
    if app.rooms_tool.members.is_some() {
        draw_members(f, app, area);
    } else if !app.rooms_tool.leaving_items.is_empty() {
        draw_leaving(f, app, area);
    } else if app.rooms_tool.leave_select {
        draw_leave_summary(f, app, area);
    } else {
        let filtered = app.rooms_tool.filtered_rooms();
        if let Some(room) = filtered.get(app.rooms_tool.selected) {
            draw_detail(f, app, area, room.id.clone());
        } else {
            f.render_widget(
                Paragraph::new("No room selected")
                    .style(Style::default().fg(MUTED))
                    .alignment(Alignment::Center),
                area,
            );
        }
    }
}

fn fmt_members(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.0}M", n as f64 / 1_000_000.0)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

fn fmt_last_active(s: &str) -> String {
    // Truncate or pad to 4 chars for right-aligned column
    if s.len() > 5 {
        s[..5].to_owned()
    } else {
        s.to_owned()
    }
}

fn draw_list_panel(f: &mut Frame, app: &App, area: Rect) {
    // Filter bar is always visible (2 rows: content + bottom border).
    let [filter_area, list_area] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(area);

    draw_filter_row(f, app, filter_area);

    if app.rooms_tool.rooms.is_empty() {
        f.render_widget(
            Paragraph::new("\n  No rooms — press r to sync")
                .style(Style::default().fg(MUTED)),
            list_area,
        );
        return;
    }

    let filtered = app.rooms_tool.filtered_rooms();
    let leave_select = app.rooms_tool.leave_select;
    let sel_idx = if filtered.is_empty() {
        None
    } else {
        Some(app.rooms_tool.selected.min(filtered.len() - 1))
    };

    if filtered.is_empty() {
        f.render_widget(
            Paragraph::new("\n  No rooms match the filter")
                .style(Style::default().fg(MUTED)),
            list_area,
        );
        return;
    }

    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let is_selected = sel_idx == Some(i);

            let indicator = if is_selected {
                Span::styled("▌", Style::default().fg(ACCENT))
            } else {
                Span::styled(" ", Style::default().fg(MUTED))
            };

            let avatar_str = format!("[{}]", r.avatar_letter);
            let avatar_span = if is_selected {
                Span::styled(avatar_str, Style::default().fg(ACCENT))
            } else {
                Span::styled(avatar_str, Style::default().fg(MUTED))
            };

            let name_style = if is_selected {
                Style::default().fg(ACCENT_DIM).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG2)
            };
            let name_str = if r.display_name.chars().count() > 28 {
                format!("{}…", &r.display_name[..27])
            } else {
                r.display_name.clone()
            };
            let name_span = Span::styled(name_str, name_style);

            let enc_span = if r.encrypted {
                Span::styled(" ●", Style::default().fg(ACCENT))
            } else {
                Span::raw("  ")
            };

            let dm_span = if r.is_dm {
                Span::styled(" DM", Style::default().fg(MUTED))
            } else {
                Span::raw("   ")
            };

            let mc_str = fmt_members(r.member_count);
            let mc_span = Span::styled(
                format!("  {:>5}", mc_str),
                Style::default().fg(MUTED),
            );

            let unread_span = if r.unread > 0 {
                Span::styled(
                    format!("  {:>3}", r.unread.min(9999)),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled("    ·", Style::default().fg(MUTED))
            };

            let last_str = r.last_active.as_deref()
                .map(fmt_last_active)
                .unwrap_or_default();
            let last_span = Span::styled(
                format!("  {:>4}", last_str),
                Style::default().fg(MUTED),
            );

            let check_span = if leave_select {
                if app.rooms_tool.checked.contains(&r.id) {
                    Span::styled("  [✓]", Style::default().fg(DANGER).add_modifier(Modifier::BOLD))
                } else {
                    Span::styled("  [ ]", Style::default().fg(MUTED))
                }
            } else {
                Span::raw("")
            };

            ListItem::new(Line::from(vec![
                indicator,
                Span::raw(" "),
                avatar_span,
                Span::raw(" "),
                name_span,
                enc_span,
                dm_span,
                mc_span,
                unread_span,
                last_span,
                check_span,
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(BG3))
        .highlight_symbol("");

    let mut state = ListState::default();
    state.select(sel_idx);

    if let Some(err) = &app.rooms_tool.error {
        let [main, err_area] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(list_area);
        f.render_stateful_widget(list, main, &mut state);
        f.render_widget(
            Paragraph::new(err.as_str())
                .style(Style::default().fg(DANGER))
                .alignment(Alignment::Center),
            err_area,
        );
    } else {
        f.render_stateful_widget(list, list_area, &mut state);
    }
}

fn draw_filter_row(f: &mut Frame, app: &App, area: Rect) {
    let filter = &app.rooms_tool.filter;
    let filtered_count = app.rooms_tool.filtered_rooms().len();
    let total = app.rooms_tool.rooms.len();

    let count_str = if !filter.input.is_empty() && filtered_count != total {
        format!("{}  {}  ", filtered_count, total)
    } else {
        format!("{}  ", total)
    };

    // Content row (top row of the 2-row area).
    let content_row = Rect::new(area.x, area.y, area.width, 1);
    let border_row = Rect::new(area.x, area.y + 1, area.width, 1);

    let bg = if filter.active { BG2 } else { BG };

    let left_line = if filter.active {
        Line::from(vec![
            Span::raw(" "),
            Span::styled("FILTER  ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD).bg(bg)),
            Span::styled(filter.input.clone(), Style::default().fg(ratatui::style::Color::White).bg(bg)),
            Span::styled("█", Style::default().fg(ACCENT).bg(bg)),
        ])
    } else if !filter.input.is_empty() {
        Line::from(vec![
            Span::raw(" "),
            Span::styled("FILTER  ", Style::default().fg(ACCENT_DIM).bg(bg)),
            Span::styled(filter.input.clone(), Style::default().fg(FG2).bg(bg)),
            Span::styled("  Esc to clear", Style::default().fg(MUTED2).bg(bg)),
        ])
    } else {
        Line::from(vec![
            Span::raw(" "),
            Span::styled("FILTER  ", Style::default().fg(MUTED).bg(bg)),
            Span::styled("press / to search", Style::default().fg(MUTED2).bg(bg)),
        ])
    };

    f.render_widget(Paragraph::new(left_line).style(Style::default().bg(bg)), content_row);
    f.render_widget(
        Paragraph::new(count_str)
            .style(Style::default().fg(MUTED).bg(bg))
            .alignment(Alignment::Right),
        content_row,
    );

    let sep_str = "─".repeat(area.width as usize);
    f.render_widget(
        Paragraph::new(sep_str).style(Style::default().fg(BORDER)),
        border_row,
    );
}

fn draw_leaving(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .rooms_tool
        .leaving_items
        .iter()
        .map(|item| {
            let (status_span, name_color) = match &item.status {
                LeaveStatus::InProgress => (
                    Span::styled("  ⟳ ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
                    ratatui::style::Color::White,
                ),
                LeaveStatus::Done => (
                    Span::styled("  ✓ ", Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)),
                    MUTED,
                ),
                LeaveStatus::Failed(_) => (
                    Span::styled("  ✗ ", Style::default().fg(DANGER).add_modifier(Modifier::BOLD)),
                    DANGER,
                ),
            };
            let mut spans = vec![
                status_span,
                Span::styled(item.room_name.clone(), Style::default().fg(name_color)),
            ];
            if let LeaveStatus::Failed(ref e) = item.status {
                spans.push(Span::styled(
                    format!("  {e}"),
                    Style::default().fg(DANGER),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let all_done = app
        .rooms_tool
        .leaving_items
        .iter()
        .all(|i| matches!(i.status, LeaveStatus::Done | LeaveStatus::Failed(_)));

    let title = if all_done {
        " Done — press Esc/q to continue "
    } else {
        " Leaving rooms… "
    };

    f.render_widget(
        List::new(items).block(
            Block::default()
                .title(Span::styled(title, Style::default().fg(ACCENT)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ACCENT)),
        ),
        area,
    );
}

fn draw_leave_summary(f: &mut Frame, app: &App, area: Rect) {
    let checked_rooms: Vec<&RoomInfo> = app
        .rooms_tool
        .filtered_rooms()
        .into_iter()
        .filter(|r| app.rooms_tool.checked.contains(&r.id))
        .collect();

    let count = checked_rooms.len();
    let mut lines = vec![
        Line::from(""),
        Line::from(if count == 0 {
            vec![Span::styled("  nothing selected", Style::default().fg(MUTED))]
        } else {
            vec![
                Span::styled(format!("  {} ", count), Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
                Span::styled("room(s) selected", Style::default().fg(FG2)),
            ]
        }),
        Line::from(""),
    ];
    for room in &checked_rooms {
        lines.push(Line::from(vec![
            Span::styled("  ☑ ", Style::default().fg(ACCENT)),
            Span::styled(room.display_name.clone(), Style::default().fg(FG2)),
        ]));
    }

    f.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(Span::styled(" leave select ", Style::default().fg(DANGER)))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(BORDER)),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_detail(f: &mut Frame, app: &App, area: Rect, room_id: String) {
    let Some(room) = app.rooms_tool.rooms.iter().find(|r| r.id == room_id) else {
        return;
    };
    let detail = &app.rooms_tool.detail;

    // Confirm-leave dialog overlays everything.
    if detail.confirm_leave {
        draw_detail_info(f, app, area, room);
        draw_confirm_leave(f, room.display_name.as_str());
        return;
    }

    // Edit mode: show edit boxes.
    if detail.editing.is_some() {
        draw_detail_editing(f, app, area, room);
        return;
    }

    draw_detail_info(f, app, area, room);
}

fn draw_detail_info(f: &mut Frame, app: &App, area: Rect, room: &crate::matrix::RoomInfo) {
    use crate::tools::{FG, FG2};

    // Add left + right horizontal padding.
    let inner = Rect::new(
        area.x + 2,
        area.y,
        area.width.saturating_sub(4),
        area.height,
    );

    let chunks = Layout::vertical([
        Constraint::Length(2), // [0] top padding
        Constraint::Length(1), // [1] avatar + room name
        Constraint::Length(1), // [2] matrix ID
        Constraint::Length(1), // [3] gap
        Constraint::Length(1), // [4] TOPIC label
        Constraint::Length(2), // [5] topic text (2 rows)
        Constraint::Length(1), // [6] gap
        Constraint::Length(7), // [7] field grid
        Constraint::Min(0),    // [8] flexible
        Constraint::Length(1), // [9] dashed separator
        Constraint::Length(1), // [10] hints / status
    ])
    .split(inner);

    // Avatar + room name row.
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("[{}]", room.avatar_letter),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                room.display_name.clone(),
                Style::default().fg(FG).add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[1],
    );

    // Matrix ID.
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("      "),
            Span::styled(room.id.clone(), Style::default().fg(MUTED)),
        ])),
        chunks[2],
    );

    // TOPIC label.
    f.render_widget(
        Paragraph::new("TOPIC").style(Style::default().fg(MUTED)),
        chunks[4],
    );

    // Topic text with word wrap.
    let topic = room.topic.as_deref().unwrap_or("(no topic)");
    f.render_widget(
        Paragraph::new(topic)
            .style(Style::default().fg(FG2))
            .wrap(Wrap { trim: false }),
        chunks[5],
    );

    // Field grid (7 rows, 2-column: 12ch label + value).
    let unread_str = if room.unread > 0 {
        format!(
            "{} ({} mention{})",
            room.unread,
            room.mentions,
            if room.mentions == 1 { "" } else { "s" }
        )
    } else {
        "none".to_owned()
    };
    let last_str = room
        .last_active
        .as_deref()
        .map(|s| format!("{s} ago"))
        .unwrap_or_else(|| "—".to_owned());
    let member_str = fmt_members(room.member_count);
    let kind_str = if room.is_dm { "direct" } else { "room" };
    let alias_str = room.alias.as_deref().unwrap_or("—");

    let fields: &[(&str, &str)] = &[
        ("NAME", room.display_name.as_str()),
        ("MATRIX ID", room.id.as_str()),
        ("ALIAS", alias_str),
        ("MEMBERS", member_str.as_str()),
        ("KIND", kind_str),
        ("ENCRYPTED", if room.encrypted { "yes" } else { "no" }),
        ("UNREAD", unread_str.as_str()),
    ];

    for (i, (label, value)) in fields.iter().enumerate() {
        let row = Rect::new(chunks[7].x, chunks[7].y + i as u16, chunks[7].width, 1);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("{:<12}", label), Style::default().fg(MUTED)),
                Span::styled((*value).to_owned(), Style::default().fg(FG2)),
            ])),
            row,
        );
    }

    // Dashed separator.
    let sep = "─".repeat(inner.width as usize);
    f.render_widget(
        Paragraph::new(sep).style(Style::default().fg(BORDER)),
        chunks[9],
    );

    // Hints row (or save/error status when editing feedback is present).
    let status_line = if let Some(ok) = &app.rooms_tool.detail.success {
        Some(Paragraph::new(ok.clone()).style(Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)))
    } else if let Some(err) = &app.rooms_tool.detail.error {
        Some(Paragraph::new(err.clone()).style(Style::default().fg(DANGER)))
    } else {
        None
    };

    if let Some(p) = status_line {
        f.render_widget(p, chunks[10]);
    } else if app.rooms_tool.detail_open {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("x", Style::default().fg(DANGER)),
                Span::styled(" leave  ", Style::default().fg(MUTED)),
                Span::styled("m", Style::default().fg(ACCENT)),
                Span::styled(" members  ", Style::default().fg(MUTED)),
                Span::styled("e", Style::default().fg(ACCENT)),
                Span::styled(" edit  ", Style::default().fg(MUTED)),
                Span::styled("Esc", Style::default().fg(ACCENT)),
                Span::styled(" back  ", Style::default().fg(MUTED)),
                Span::styled(last_str.as_str(), Style::default().fg(MUTED)),
            ])),
            chunks[10],
        );
    } else {
        f.render_widget(
            Paragraph::new(Span::styled(
                "Enter to open room",
                Style::default().fg(MUTED2),
            )),
            chunks[10],
        );
    }
}

fn draw_detail_editing(f: &mut Frame, app: &App, area: Rect, room: &crate::matrix::RoomInfo) {
    use crate::tools::FG;

    let detail = &app.rooms_tool.detail;

    let field_text = |field: DetailField, fallback: &str| -> String {
        if detail.editing.is_some() && detail.focused == field {
            detail.editing.as_deref().unwrap_or("").to_owned()
        } else {
            fallback.to_owned()
        }
    };

    let name_text = field_text(DetailField::Name, &room.display_name);
    let topic_text = field_text(DetailField::Topic, room.topic.as_deref().unwrap_or(""));
    let alias_text = field_text(DetailField::Alias, room.alias.as_deref().unwrap_or(""));

    let make_field = |label: &str, value: &str, focused: bool, editing: bool| -> Paragraph<'static> {
        let border_color = if editing { ACCENT_DIM } else if focused { ACCENT } else { BORDER };
        let text_color = if focused || editing { FG } else { MUTED };
        let placeholder = if !editing && value.is_empty() {
            "(none)".to_owned()
        } else if editing {
            format!("{value}█")
        } else {
            value.to_owned()
        };
        Paragraph::new(placeholder)
            .style(Style::default().fg(text_color))
            .block(
                Block::default()
                    .title(Span::styled(
                        format!(" {label} "),
                        Style::default().fg(border_color),
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color)),
            )
    };

    let editing = detail.editing.is_some();
    let focused = detail.focused;

    let inner = Rect::new(area.x + 2, area.y, area.width.saturating_sub(4), area.height);
    let chunks = Layout::vertical([
        Constraint::Length(1),  // padding
        Constraint::Length(3),  // name
        Constraint::Length(3),  // topic
        Constraint::Length(3),  // alias
        Constraint::Length(1),  // gap
        Constraint::Length(1),  // status
        Constraint::Min(0),
    ])
    .split(inner);

    f.render_widget(
        make_field("Name", &name_text, focused == DetailField::Name, editing && focused == DetailField::Name),
        chunks[1],
    );
    f.render_widget(
        make_field("Topic", &topic_text, focused == DetailField::Topic, editing && focused == DetailField::Topic),
        chunks[2],
    );
    f.render_widget(
        make_field("Alias", &alias_text, focused == DetailField::Alias, editing && focused == DetailField::Alias),
        chunks[3],
    );

    let status: Paragraph = if detail.saving {
        Paragraph::new("Saving…")
            .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
            .alignment(Alignment::Center)
    } else if let Some(err) = &detail.error {
        Paragraph::new(err.as_str())
            .style(Style::default().fg(DANGER))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
    } else if let Some(ok) = &detail.success {
        Paragraph::new(ok.as_str())
            .style(Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD))
            .alignment(Alignment::Center)
    } else {
        Paragraph::new("")
    };
    f.render_widget(status, chunks[5]);
}

fn draw_confirm_leave(f: &mut Frame, room_name: &str) {
    let area = f.area();
    let popup = centered_rect(54, 7, area);
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            Line::from(vec![
                Span::raw("  Leave "),
                Span::styled(room_name.to_owned(), Style::default().fg(ACCENT_DIM).add_modifier(Modifier::BOLD)),
                Span::raw("?"),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  y/Enter", Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)),
                Span::raw("  confirm    "),
                Span::styled("any other key", Style::default().fg(DANGER).add_modifier(Modifier::BOLD)),
                Span::raw("  cancel"),
            ]),
        ])
        .block(
            Block::default()
                .title(Span::styled(" Confirm ", Style::default().fg(DANGER).add_modifier(Modifier::BOLD)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DANGER))
                .style(Style::default().bg(ratatui::style::Color::Rgb(25, 15, 15))),
        )
        .wrap(Wrap { trim: false }),
        popup,
    );
}

fn draw_members(f: &mut Frame, app: &App, area: Rect) {
    let Some(ms) = &app.rooms_tool.members else { return; };

    if ms.loading {
        f.render_widget(
            Paragraph::new("Loading members…")
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    // Split off a bottom row for the pl_edit prompt or error.
    let has_prompt = ms.pl_edit.is_some();
    let has_error = ms.error.is_some();
    let bottom_rows = has_prompt as u16 + has_error as u16;
    let chunks = if bottom_rows > 0 {
        Layout::vertical([Constraint::Min(1), Constraint::Length(bottom_rows)]).split(area)
    } else {
        Layout::vertical([Constraint::Min(1)]).split(area)
    };
    let list_area = chunks[0];

    if ms.members.is_empty() {
        f.render_widget(
            Paragraph::new("No members found.")
                .style(Style::default().fg(MUTED))
                .alignment(Alignment::Center),
            list_area,
        );
    } else {
        let items: Vec<ListItem> = ms
            .members
            .iter()
            .map(|m| {
                let name = m.display_name.as_deref().unwrap_or(&m.user_id).to_owned();
                let uid_str = if m.display_name.is_some() {
                    format!("  {}", m.user_id)
                } else {
                    String::new()
                };
                let self_tag = if m.is_self {
                    Span::styled(" (you)", Style::default().fg(MUTED))
                } else {
                    Span::raw("")
                };
                let pl_color = if m.power_level >= 75 {
                    SUCCESS
                } else if m.power_level >= 25 {
                    ACCENT
                } else {
                    MUTED
                };
                let name_color = if m.is_self {
                    MUTED
                } else {
                    ratatui::style::Color::White
                };
                ListItem::new(Line::from(vec![
                    Span::styled(name, Style::default().fg(name_color)),
                    self_tag,
                    Span::styled(uid_str, Style::default().fg(MUTED)),
                    Span::styled(
                        format!("  [{}]", m.power_level),
                        Style::default().fg(pl_color),
                    ),
                ]))
            })
            .collect();

        let room_name = app.rooms_tool
            .selected_room_idx()
            .and_then(|i| app.rooms_tool.rooms.get(i))
            .map(|r| r.display_name.as_str())
            .unwrap_or("Room");

        let list = List::new(items)
            .block(
                Block::default()
                    .title(Span::styled(
                        format!(" {} — {} member(s) ", room_name, ms.members.len()),
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
        state.select(Some(ms.selected.min(ms.members.len().saturating_sub(1))));
        f.render_stateful_widget(list, list_area, &mut state);
    }

    // Bottom area: pl_edit prompt then error.
    if bottom_rows > 0 {
        let mut sub = chunks[1];
        if has_prompt {
            let row = Rect::new(sub.x, sub.y, sub.width, 1);
            let input = ms.pl_edit.as_deref().unwrap_or("");
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(" Set power level: ", Style::default().fg(ACCENT_DIM)),
                    Span::styled(input.to_owned(), Style::default().fg(ratatui::style::Color::White)),
                    Span::styled("█", Style::default().fg(ACCENT_DIM)),
                ])),
                row,
            );
            sub = Rect::new(sub.x, sub.y + 1, sub.width, sub.height.saturating_sub(1));
        }
        if has_error {
            if let Some(err) = &ms.error {
                f.render_widget(
                    Paragraph::new(err.as_str())
                        .style(Style::default().fg(DANGER))
                        .alignment(Alignment::Center),
                    Rect::new(sub.x, sub.y, sub.width, 1),
                );
            }
        }
    }

    if let Some((action, user_id)) = &ms.confirm {
        draw_mod_confirm(f, *action, user_id);
    }
}

fn draw_mod_confirm(f: &mut Frame, action: ModAction, user_id: &str) {
    let area = f.area();
    let popup = centered_rect(54, 7, area);
    f.render_widget(Clear, popup);
    let verb = match action {
        ModAction::Kick => "Kick",
        ModAction::Ban => "Ban",
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            Line::from(vec![
                Span::raw(format!("  {verb} ")),
                Span::styled(user_id.to_owned(), Style::default().fg(ACCENT_DIM).add_modifier(Modifier::BOLD)),
                Span::raw("?"),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  y/Enter", Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)),
                Span::raw("  confirm    "),
                Span::styled("any other key", Style::default().fg(DANGER).add_modifier(Modifier::BOLD)),
                Span::raw("  cancel"),
            ]),
        ])
        .block(
            Block::default()
                .title(Span::styled(format!(" {verb} Member "), Style::default().fg(DANGER).add_modifier(Modifier::BOLD)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DANGER))
                .style(Style::default().bg(ratatui::style::Color::Rgb(25, 15, 15))),
        )
        .wrap(Wrap { trim: false }),
        popup,
    );
}

// ---------------------------------------------------------------------------
// Hint spans
// ---------------------------------------------------------------------------

pub fn hint_spans(app: &App) -> Vec<Span<'static>> {
    if let Some(ms) = &app.rooms_tool.members {
        if ms.pl_edit.is_some() {
            return vec![
                Span::styled("Enter", Style::default().fg(SUCCESS)),
                Span::raw(" set power level  "),
                Span::styled("Esc", Style::default().fg(ACCENT)),
                Span::raw(" cancel"),
            ];
        }
        return vec![
            Span::styled("j/k", Style::default().fg(ACCENT)),
            Span::raw(" navigate  "),
            Span::styled("p", Style::default().fg(ACCENT)),
            Span::raw(" power level  "),
            Span::styled("K", Style::default().fg(DANGER)),
            Span::raw(" kick  "),
            Span::styled("b", Style::default().fg(DANGER)),
            Span::raw(" ban  "),
            Span::styled("r", Style::default().fg(ACCENT)),
            Span::raw(" refresh  "),
            Span::styled("Esc/q", Style::default().fg(ACCENT)),
            Span::raw(" back"),
        ];
    }
    if app.rooms_tool.detail.editing.is_some() {
        return vec![
            Span::styled("Tab", Style::default().fg(ACCENT)),
            Span::raw(" next field  "),
            Span::styled("Enter", Style::default().fg(SUCCESS)),
            Span::raw(" save  "),
            Span::styled("Esc", Style::default().fg(ACCENT)),
            Span::raw(" discard"),
        ];
    }
    if app.rooms_tool.leave_select {
        return vec![
            Span::styled("j/k", Style::default().fg(ACCENT)),
            Span::raw(" navigate  "),
            Span::styled("Space", Style::default().fg(ACCENT)),
            Span::raw(" select  "),
            Span::styled("Enter", Style::default().fg(DANGER)),
            Span::raw(" leave selected  "),
            Span::styled("Esc", Style::default().fg(ACCENT)),
            Span::raw(" cancel"),
        ];
    }
    if app.rooms_tool.detail_open {
        return vec![
            Span::styled("j/k", Style::default().fg(ACCENT)),
            Span::raw(" navigate  "),
            Span::styled("x", Style::default().fg(DANGER)),
            Span::raw(" leave  "),
            Span::styled("m", Style::default().fg(ACCENT)),
            Span::raw(" members  "),
            Span::styled("e", Style::default().fg(ACCENT)),
            Span::raw(" edit  "),
            Span::styled("Esc", Style::default().fg(ACCENT)),
            Span::raw(" back"),
        ];
    }
    vec![
        Span::styled("j/k", Style::default().fg(ACCENT)),
        Span::raw(" navigate  "),
        Span::styled("/", Style::default().fg(ACCENT)),
        Span::raw(" filter  "),
        Span::styled("Enter", Style::default().fg(ACCENT)),
        Span::raw(" open  "),
        Span::styled("d", Style::default().fg(DANGER)),
        Span::raw(" multi-leave  "),
        Span::styled("q", Style::default().fg(ACCENT)),
        Span::raw(" back"),
    ]
}

pub fn tool_name() -> &'static str {
    "Rooms"
}
