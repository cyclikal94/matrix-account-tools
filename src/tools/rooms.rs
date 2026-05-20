use std::collections::HashSet;
use std::time::Instant;

use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap},
};
use tokio::sync::{mpsc, oneshot};

use crate::app::{ActiveTool, App};
use crate::matrix::{MemberInfo, RoomInfo};
use crate::tools::{ACCENT, ACCENT_DIM, BG, BG3, BORDER, DANGER, FG, FG2, MUTED, MUTED2, SUCCESS, FilterState, Filterable, filter_hint_spans};
use crate::tools::common::{copy_to_clipboard, handle_filter_keys};
use crate::ui::centered_rect;

impl Filterable for RoomInfo {
    fn filter_cols() -> &'static [&'static str] { &["all", "name", "dm", "enc"] }
    fn filter_value(&self, col: usize) -> String {
        match col {
            1 => self.display_name.clone(),
            2 => if self.is_dm { "dm".to_owned() } else { "group".to_owned() },
            3 => if self.encrypted { "encrypted".to_owned() } else { "unencrypted".to_owned() },
            _ => String::new(),
        }
    }
}

impl Filterable for MemberInfo {
    fn filter_cols() -> &'static [&'static str] { &["all", "name", "id", "pl"] }
    fn filter_value(&self, col: usize) -> String {
        match col {
            1 => self.display_name.clone().unwrap_or_else(|| self.user_id.clone()),
            2 => self.user_id.clone(),
            3 => self.power_level.to_string(),
            _ => String::new(),
        }
    }
}

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
    // Tagged with (room_id, message, timestamp) — shown for 10 s, only for the matching room.
    pub error: Option<(String, String, Instant)>,
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

// ---------------------------------------------------------------------------
// Member profile popup state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MemberProfileState {
    pub user_id: String,
    pub display_name: Option<String>,
    pub power_level: i64,
    pub is_self: bool,
    pub confirm_ignore: bool,
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
    pub filter: FilterState,
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
    pub detail_open: bool, // true = user has "entered" the room and can navigate fields

    // Members: auto-loaded whenever a room is navigated to; detail_members_focused = true
    // means j/k/p/K/b operate on the inline member list rather than the detail fields.
    pub members: Option<MembersState>,
    pub detail_members_focused: bool,
    pub topic_scroll: u16,

    // Member profile popup (overlay on top of the detail panel).
    pub member_profile: Option<MemberProfileState>,
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
            detail_open: false,
            members: None,
            detail_members_focused: false,
            topic_scroll: 0,
            member_profile: None,
        }
    }
}

impl RoomBrowserState {
    pub fn filtered_rooms(&self) -> Vec<&RoomInfo> {
        self.rooms.iter().filter(|r| self.filter.matches_item(*r)).collect()
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

fn filtered_members_vec(ms: &MembersState) -> Vec<&MemberInfo> {
    ms.members.iter().filter(|m| ms.filter.matches_item(*m)).collect()
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
    // Member profile popup (overlay) takes priority.
    if app.rooms_tool.member_profile.is_some() {
        handle_member_profile(app, code).await;
        return;
    }

    // Members focused inline in detail panel — j/k/p/K/b operate on member list.
    if app.rooms_tool.detail_open && app.rooms_tool.detail_members_focused {
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

    // Room detail view (entered via e/Enter from list).
    if app.rooms_tool.detail_open {
        handle_detail_view(app, code).await;
        return;
    }

    // Normal list.
    handle_list(app, code).await;
}

fn handle_filter_input(app: &mut App, code: KeyCode) {
    let filtered_len = app.rooms_tool.filtered_rooms().len();
    handle_filter_keys(
        &mut app.rooms_tool.filter,
        &mut app.rooms_tool.selected,
        filtered_len,
        RoomInfo::filter_cols().len(),
        code,
    );
}

async fn handle_list(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.active_tool = ActiveTool::Home,
        KeyCode::Char('j') | KeyCode::Down => nav_down(app),
        KeyCode::Char('k') | KeyCode::Up => nav_up(app),
        KeyCode::Char('/') => {
            app.rooms_tool.filter.active = true;
            app.rooms_tool.filter.input.clear();
            app.rooms_tool.selected = 0;
        }
        KeyCode::Char('e') | KeyCode::Char('d') | KeyCode::Enter => {
            if !app.rooms_tool.filtered_rooms().is_empty() {
                app.rooms_tool.detail_open = true;
                app.rooms_tool.detail_members_focused = false;
                app.rooms_tool.detail.focused = DetailField::Name;
            }
        }
        KeyCode::Char('m') | KeyCode::Char('M') => {
            if !app.rooms_tool.filtered_rooms().is_empty() {
                app.rooms_tool.detail_open = true;
                app.rooms_tool.detail_members_focused = true;
            }
        }
        KeyCode::Char('x') | KeyCode::Char('X') => {
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
        KeyCode::Esc | KeyCode::Char('q') => {
            app.rooms_tool.detail_open = false;
            app.rooms_tool.detail_members_focused = false;
            app.rooms_tool.topic_scroll = 0;
            app.rooms_tool.member_profile = None;
        }
        KeyCode::Char('i') => {
            if let Some(room_id) = app.rooms_tool.selected_room_id() {
                copy_to_clipboard(&room_id);
                app.toast = Some(("Room ID copied".to_owned(), SUCCESS, Instant::now()));
            }
        }
        // j/k navigate fields in visual order: Name → Topic → Alias
        KeyCode::Char('j') | KeyCode::Down => {
            app.rooms_tool.detail.focused = match app.rooms_tool.detail.focused {
                DetailField::Name => DetailField::Topic,
                DetailField::Topic => DetailField::Alias,
                DetailField::Alias => DetailField::Alias,
            };
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.rooms_tool.detail.focused = match app.rooms_tool.detail.focused {
                DetailField::Name => DetailField::Name,
                DetailField::Topic => DetailField::Name,
                DetailField::Alias => DetailField::Topic,
            };
        }
        KeyCode::PageDown => {
            app.rooms_tool.topic_scroll = app.rooms_tool.topic_scroll.saturating_add(1);
        }
        KeyCode::PageUp => {
            app.rooms_tool.topic_scroll = app.rooms_tool.topic_scroll.saturating_sub(1);
        }
        KeyCode::Char('e') | KeyCode::Enter => {
            if let Some(idx) = app.rooms_tool.selected_room_idx() {
                if let Some(room) = app.rooms_tool.rooms.get(idx) {
                    let current = match app.rooms_tool.detail.focused {
                        DetailField::Name => room.display_name.clone(),
                        DetailField::Topic => room.topic.clone().unwrap_or_default(),
                        DetailField::Alias => room.aliases.first().cloned().unwrap_or_default(),
                    };
                    app.rooms_tool.detail.editing = Some(current);
                    app.rooms_tool.detail.error = None;
                }
            }
        }
        KeyCode::Char('m') | KeyCode::Char('M') => {
            // Jump focus into the inline member list.
            app.rooms_tool.detail_members_focused = true;
        }
        KeyCode::Char('x') | KeyCode::Char('X') => {
            if !app.rooms_tool.rooms.is_empty() {
                app.rooms_tool.detail.confirm_leave = true;
            }
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
                let filtered_members = filtered_members_vec(ms);
                let user_id = filtered_members.get(ms.selected).map(|m| m.user_id.clone());
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

    // Members filter popup active.
    if ms.filter.active {
        let filtered_len = filtered_members_vec(app.rooms_tool.members.as_ref().unwrap()).len();
        let ms_mut = app.rooms_tool.members.as_mut().unwrap();
        handle_filter_keys(
            &mut ms_mut.filter,
            &mut ms_mut.selected,
            filtered_len,
            MemberInfo::filter_cols().len(),
            code,
        );
        return;
    }

    if ms.loading { return; }

    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            if !app.rooms_tool.members.as_ref().map_or(true, |m| m.filter.input.is_empty()) {
                app.rooms_tool.members.as_mut().unwrap().filter.clear();
            } else {
                // Esc from members → close detail entirely, back to room list.
                app.rooms_tool.detail_open = false;
                app.rooms_tool.detail_members_focused = false;
                app.rooms_tool.topic_scroll = 0;
                app.rooms_tool.member_profile = None;
            }
        }
        KeyCode::Char('/') => {
            app.rooms_tool.members.as_mut().unwrap().filter.active = true;
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            // d → back to room detail view (members stay loaded).
            app.rooms_tool.detail_members_focused = false;
        }
        KeyCode::Enter | KeyCode::Char('e') => {
            let ms = app.rooms_tool.members.as_ref().unwrap();
            let filtered_members = filtered_members_vec(ms);
            if let Some(m) = filtered_members.get(ms.selected) {
                let profile = MemberProfileState {
                    user_id: m.user_id.clone(),
                    display_name: m.display_name.clone(),
                    power_level: m.power_level,
                    is_self: m.is_self,
                    confirm_ignore: false,
                };
                app.rooms_tool.member_profile = Some(profile);
            }
        }
        KeyCode::Char('i') => {
            let ms = app.rooms_tool.members.as_ref().unwrap();
            let filtered_members = filtered_members_vec(ms);
            if let Some(m) = filtered_members.get(ms.selected) {
                let uid = m.user_id.clone();
                copy_to_clipboard(&uid);
                app.toast = Some((format!("Copied {uid}"), SUCCESS, Instant::now()));
            }
        }
        KeyCode::Char('I') => {
            let ms = app.rooms_tool.members.as_ref().unwrap();
            let filtered_members = filtered_members_vec(ms);
            if let Some(m) = filtered_members.get(ms.selected) {
                if !m.is_self {
                    let profile = MemberProfileState {
                        user_id: m.user_id.clone(),
                        display_name: m.display_name.clone(),
                        power_level: m.power_level,
                        is_self: m.is_self,
                        confirm_ignore: true,
                    };
                    app.rooms_tool.member_profile = Some(profile);
                } else {
                    app.rooms_tool.members.as_mut().unwrap().error =
                        Some("Cannot ignore yourself.".to_owned());
                }
            }
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let ms = app.rooms_tool.members.as_ref().unwrap();
            let filtered_len = filtered_members_vec(ms).len();
            let ms_mut = app.rooms_tool.members.as_mut().unwrap();
            if ms_mut.selected + 1 < filtered_len {
                ms_mut.selected += 1;
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
            let filtered_members = filtered_members_vec(ms);
            if let Some(m) = filtered_members.get(ms.selected) {
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
            let filtered_members = filtered_members_vec(ms);
            if let Some(m) = filtered_members.get(ms.selected) {
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
            let filtered_members = filtered_members_vec(ms);
            if let Some(m) = filtered_members.get(ms.selected) {
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
        on_room_changed(app);
    }
}

fn nav_up(app: &mut App) {
    if app.rooms_tool.selected > 0 {
        app.rooms_tool.selected -= 1;
        on_room_changed(app);
    }
}

fn on_room_changed(app: &mut App) {
    app.rooms_tool.topic_scroll = 0;
    app.rooms_tool.detail_members_focused = false;
    app.rooms_tool.members = Some(MembersState::default());
    start_member_load(app);
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
                let filtered_len = app.rooms_tool.filtered_rooms().len();
                if app.rooms_tool.selected >= filtered_len && filtered_len > 0 {
                    app.rooms_tool.selected = filtered_len - 1;
                }
                app.rooms_tool.detail = DetailState::default();
                app.rooms_tool.detail_open = false;
                app.rooms_tool.detail_members_focused = false;
                app.rooms_tool.members = None;
            }
            Err(e) => {
                app.rooms_tool.detail.error = Some((room_id.clone(), format!("Leave failed: {e}"), Instant::now()));
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
                    DetailField::Alias => {
                        if val.is_empty() {
                            if !room.aliases.is_empty() { room.aliases.remove(0); }
                        } else if room.aliases.is_empty() {
                            room.aliases.push(val);
                        } else {
                            room.aliases[0] = val;
                        }
                    }
                }
            }
            app.toast = Some(("Saved!".to_owned(), SUCCESS, Instant::now()));
        }
        Err(e) => {
            app.rooms_tool.detail.error = Some((room_id.clone(), format!("{e}"), Instant::now()));
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

async fn handle_member_profile(app: &mut App, code: KeyCode) {
    let Some(profile) = &app.rooms_tool.member_profile else { return; };

    if profile.confirm_ignore {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                let uid = profile.user_id.clone();
                do_ignore_member(app, uid).await;
            }
            _ => {
                if let Some(p) = &mut app.rooms_tool.member_profile {
                    p.confirm_ignore = false;
                }
            }
        }
        return;
    }

    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.rooms_tool.member_profile = None;
        }
        KeyCode::Char('i') => {
            let uid = profile.user_id.clone();
            copy_to_clipboard(&uid);
            app.toast = Some((format!("Copied {uid}"), SUCCESS, Instant::now()));
        }
        KeyCode::Char('I') => {
            if !profile.is_self {
                if let Some(p) = &mut app.rooms_tool.member_profile {
                    p.confirm_ignore = true;
                }
            }
        }
        _ => {}
    }
}

async fn do_ignore_member(app: &mut App, user_id: String) {
    let room_id = app.rooms_tool.selected_room_id().unwrap_or_default();
    app.rooms_tool.member_profile = None;
    if let Some(client) = &app.matrix {
        match client.ignore_user(&user_id).await {
            Ok(()) => {
                app.toast = Some((format!("Ignored {user_id}"), SUCCESS, Instant::now()));
            }
            Err(e) => {
                app.rooms_tool.detail.error =
                    Some((room_id, format!("Ignore failed: {e}"), Instant::now()));
            }
        }
    }
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
    // Immediately start loading members for the first selected room.
    app.rooms_tool.members = Some(MembersState::default());
    start_member_load(app);
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

    let cols = Layout::horizontal([
        Constraint::Percentage(50),
        Constraint::Length(1),
        Constraint::Min(20),
    ])
    .split(area);

    draw_list_panel(f, app, cols[0]);
    draw_right_panel(f, app, cols[2]);

    if app.rooms_tool.filter.active {
        crate::ui::draw_filter_popup(f, &app.rooms_tool.filter, cols[0]);
    }
}

const DETAIL_HEIGHT: u16 = 18;

fn draw_right_panel(f: &mut Frame, app: &App, area: Rect) {
    if !app.rooms_tool.leaving_items.is_empty() {
        draw_leaving(f, app, area);
        return;
    }
    if app.rooms_tool.leave_select {
        draw_leave_summary(f, app, area);
        return;
    }
    let filtered = app.rooms_tool.filtered_rooms();
    if let Some(room) = filtered.get(app.rooms_tool.selected) {
        let room_id = room.id.clone();
        let chunks = Layout::vertical([
            Constraint::Length(DETAIL_HEIGHT),
            Constraint::Min(0),
        ])
        .split(area);
        draw_detail(f, app, chunks[0], room_id);
        draw_members_block(f, app, chunks[1]);
    } else {
        f.render_widget(
            Paragraph::new("No room selected")
                .style(Style::default().fg(MUTED))
                .alignment(Alignment::Center),
            area,
        );
    }
}

/// Simulate ratatui's word-wrap line count for scroll-cap calculation.
/// Splits on whitespace (matching Wrap { trim: true }) and handles long words.
fn wrapped_line_count(text: &str, width: u16) -> u16 {
    if width == 0 { return 1; }
    let w = width as usize;
    let mut total = 0u16;
    for paragraph in text.split('\n') {
        let words: Vec<&str> = paragraph.split_whitespace().collect();
        if words.is_empty() {
            total += 1;
            continue;
        }
        let mut line_len = 0usize;
        let mut lines = 1u16;
        for word in &words {
            let wl = word.chars().count();
            if line_len == 0 {
                // Long words span multiple lines on their own.
                if wl > w {
                    lines += (wl / w) as u16;
                    line_len = wl % w;
                } else {
                    line_len = wl;
                }
            } else if line_len + 1 + wl <= w {
                line_len += 1 + wl;
            } else {
                lines += 1;
                if wl > w {
                    lines += (wl / w) as u16;
                    line_len = wl % w;
                } else {
                    line_len = wl;
                }
            }
        }
        total += lines;
    }
    total.max(1)
}

fn draw_list_panel(f: &mut Frame, app: &App, area: Rect) {
    let list_area = area;

    if app.rooms_tool.rooms.is_empty() {
        f.render_widget(
            Paragraph::new("No rooms. Press 'r' to sync.")
                .style(Style::default().fg(MUTED))
                .alignment(Alignment::Center),
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

    // Right-pin badges: enc●(1) + " "(1) + "DM"/"  "(2) = 4 chars. Check adds 4 more.
    // inner_w accounts for 2 borders + 2 horizontal padding = 4 total.
    let inner_w = list_area.width.saturating_sub(4) as usize;
    let fixed_left = 6usize; // indicator(2) + "[X]"(3) + " "(1)
    let badge_w = 4usize;    // ●(1) + " "(1) + "DM"/"  "(2)
    let check_w = if leave_select { 4usize } else { 0 }; // " [✓]" / " [ ]"
    let name_w = inner_w.saturating_sub(fixed_left + badge_w + check_w);

    let mut items: Vec<ListItem> = Vec::new();
    items.extend(filtered
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let is_selected = sel_idx == Some(i);

            let indicator = if is_selected {
                Span::styled("▌ ", Style::default().fg(ACCENT))
            } else {
                Span::styled("  ", Style::default().fg(MUTED))
            };

            let avatar_str = format!("[{}]", r.avatar_letter);
            let avatar_span = if is_selected {
                Span::styled(avatar_str, Style::default().fg(ACCENT).bg(BG3))
            } else {
                Span::styled(avatar_str, Style::default().fg(MUTED2))
            };

            let name_style = if is_selected {
                Style::default().fg(ACCENT_DIM).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG2)
            };
            // Pad name to name_w so right badges are pinned to the right edge.
            let name_str = if name_w == 0 {
                String::new()
            } else {
                let char_count = r.display_name.chars().count();
                if char_count > name_w {
                    let truncated: String = r.display_name.chars().take(name_w.saturating_sub(1)).collect();
                    format!("{truncated}…")
                } else {
                    format!("{:<1$}", r.display_name, name_w)
                }
            };

            // DM tag or blank spacer.
            let dm_span = if r.is_dm {
                Span::styled("dm ", Style::default().fg(MUTED))
            } else {
                Span::raw("   ")
            };

            // Encryption badge — green = encrypted, grey = unencrypted.
            let enc_span = if r.encrypted {
                Span::styled("●", Style::default().fg(ACCENT))
            } else {
                Span::styled("●", Style::default().fg(MUTED))
            };

            // Checkbox for leave-select.
            let check_span = if leave_select {
                if app.rooms_tool.checked.contains(&r.id) {
                    Span::styled(" [✓]", Style::default().fg(DANGER).add_modifier(Modifier::BOLD))
                } else {
                    Span::styled(" [ ]", Style::default().fg(MUTED))
                }
            } else {
                Span::raw("")
            };

            ListItem::new(Line::from(vec![
                indicator,
                avatar_span,
                Span::raw(" "),
                Span::styled(name_str, name_style),
                dm_span,
                enc_span,
                check_span,
            ]))
        }));

    let mut state = ListState::default();
    state.select(sel_idx);

    let total_rooms = app.rooms_tool.rooms.len();
    let filtered_count = filtered.len();
    let title = if leave_select {
        format!(
            " Room List ({}) — {} selected — Enter to leave ",
            total_rooms,
            app.rooms_tool.checked.len()
        )
    } else if !app.rooms_tool.filter.input.is_empty() {
        format!(" Room List ({filtered_count}/{total_rooms}) ")
    } else {
        format!(" Room List ({total_rooms}) ")
    };

    let list_focused = !app.rooms_tool.detail_open && !leave_select;
    let border_color = if leave_select { DANGER } else if list_focused { ACCENT } else { BORDER };
    let title_color = if leave_select { DANGER } else if list_focused { ACCENT } else { MUTED };

    let list = List::new(items)
        .block(
            Block::default()
                .title(Span::styled(title, Style::default().fg(title_color)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .padding(Padding::new(1, 1, 1, 1)),
        )
        .highlight_style(Style::default().bg(BG3))
        .highlight_symbol("");

    if let Some(err) = &app.rooms_tool.error {
        let ec = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(list_area);
        f.render_stateful_widget(list, ec[0], &mut state);
        f.render_widget(
            Paragraph::new(err.as_str())
                .style(Style::default().fg(DANGER))
                .alignment(Alignment::Center),
            ec[1],
        );
    } else {
        f.render_stateful_widget(list, list_area, &mut state);
    }
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

    let detail_active = app.rooms_tool.detail_open
        && !app.rooms_tool.detail_members_focused
        && app.rooms_tool.member_profile.is_none();
    let border_color = if detail_active { ACCENT } else { BORDER };
    let outer_block = Block::default()
        .title(Span::styled(" Room Details ", Style::default().fg(border_color)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));
    let inner_area = outer_block.inner(area);
    f.render_widget(outer_block, area);
    let area = inner_area;

    let editing = detail.editing.is_some();
    let focused = detail.focused;
    let detail_open = app.rooms_tool.detail_open;

    let cx = area.x + 1;
    let cw = area.width.saturating_sub(2);
    if cw < 4 { return; }

    // Build values — cursor injected when editing that field.
    let edit_val = |field: DetailField, fallback: &str| -> String {
        if editing && focused == field {
            format!("{}█", detail.editing.as_deref().unwrap_or(""))
        } else {
            fallback.to_owned()
        }
    };
    let topic_val  = edit_val(DetailField::Topic, room.topic.as_deref().unwrap_or(""));
    let all_aliases_str = room.aliases.join(", ");
    let alias_val  = edit_val(DetailField::Alias, &all_aliases_str);
    let kind_str   = if room.is_dm { "DM" } else { "public" };
    let enc_str    = if room.encrypted { "yes" } else { "no" };
    let unread_str = if room.unread > 0 || room.mentions > 0 {
        format!("{} ({} mentions)", room.unread, room.mentions)
    } else { "0".to_owned() };
    let last_str   = room.last_active.as_deref()
        .map(|s| format!("{s} ago"))
        .unwrap_or_else(|| "—".to_owned());

    // Topic view height: at most 3 rows (scrollable), but shrinks to actual line count so
    // short topics don't leave a large gap before the field table.
    let topic_raw = if editing && focused == DetailField::Topic {
        detail.editing.as_deref().unwrap_or("")
    } else {
        room.topic.as_deref().unwrap_or("")
    };
    let topic_view_height = wrapped_line_count(topic_raw, cw).max(1).min(3) as u16;

    let chunks = Layout::vertical([
        Constraint::Length(1), // [0] top padding
        Constraint::Length(1), // [1] avatar + name (NAME field — editable)
        Constraint::Length(1), // [2] room id (muted)
        Constraint::Length(1), // [3] blank
        Constraint::Length(1), // [4] TOPIC label
        Constraint::Length(1), // [5] blank after TOPIC label
        Constraint::Length(topic_view_height), // [6] topic text (scrollable)
        Constraint::Length(1), // [7] blank
        Constraint::Length(5), // [8] field table: ALIAS, KIND, ENCRYPTED, UNREAD, LAST ACTIVITY
        Constraint::Min(0),    // [9] remaining space / status bar row
    ])
    .split(area);

    // [1] Avatar + name (NAME field — editable when focused)
    let name_display = if editing && focused == DetailField::Name {
        format!("{}█", detail.editing.as_deref().unwrap_or(""))
    } else {
        room.display_name.clone()
    };
    let name_color = if editing && focused == DetailField::Name {
        ACCENT_DIM
    } else if detail_open && focused == DetailField::Name {
        ACCENT
    } else {
        FG
    };
    let avatar = format!(" {} ", room.avatar_letter);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(avatar, Style::default().fg(BG).bg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(name_display, Style::default().fg(name_color).add_modifier(Modifier::BOLD)),
        ])),
        Rect::new(cx, chunks[1].y, cw, 1),
    );

    // [2] Room ID (indented past avatar: 3 chars + 2 spaces = 5)
    f.render_widget(
        Paragraph::new(Span::styled(room.id.clone(), Style::default().fg(MUTED))),
        Rect::new(cx + 5, chunks[2].y, cw.saturating_sub(5), 1),
    );

    // [4] TOPIC label — accent when focused; scroll indicator on right edge
    let topic_label_color = if detail_open && focused == DetailField::Topic { ACCENT } else { MUTED };
    let topic_total_lines = wrapped_line_count(topic_raw, cw);
    let topic_max_scroll = topic_total_lines.saturating_sub(topic_view_height);
    let effective_topic_scroll = app.rooms_tool.topic_scroll.min(topic_max_scroll);
    let can_up   = effective_topic_scroll > 0;
    let can_down = effective_topic_scroll < topic_max_scroll;
    let scroll_indicator: &str = match (can_up, can_down) {
        (true,  true)  => " ▴▾",
        (true,  false) => " ▴",
        (false, true)  => " ▾",
        (false, false) => "",
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("TOPIC", Style::default().fg(topic_label_color)),
            Span::styled(scroll_indicator, Style::default().fg(MUTED)),
        ])),
        Rect::new(cx, chunks[4].y, cw, 1),
    );

    // [6] Topic text (wraps, scrollable with PageDown/PageUp) — chunk[5] is blank spacer
    let topic_display = if topic_val.is_empty() {
        if editing && focused == DetailField::Topic { "█".to_owned() } else { "(no topic)".to_owned() }
    } else { topic_val.clone() };
    let topic_color = if editing && focused == DetailField::Topic { ACCENT_DIM }
        else if detail_open && focused == DetailField::Topic { FG }
        else { FG2 };
    f.render_widget(
        Paragraph::new(topic_display)
            .style(Style::default().fg(topic_color))
            .wrap(Wrap { trim: true })
            .scroll((effective_topic_scroll, 0)),
        Rect::new(cx, chunks[6].y, cw, topic_view_height),
    );

    // [8] Field table — no NAME or MATRIX ID (shown in header above) — chunk[7] is blank spacer
    const LABEL_W: usize = 18;
    let foc = |field: DetailField| detail_open && focused == field;
    let fields: &[(&str, &str, bool)] = &[
        ("ALIAS",         &alias_val,  foc(DetailField::Alias)),
        ("KIND",          kind_str,    false),
        ("ENCRYPTED",     enc_str,     false),
        ("UNREAD",        &unread_str, false),
        ("LAST ACTIVITY", &last_str,   false),
    ];
    for (i, (label, value, is_foc)) in fields.iter().enumerate() {
        let fy = chunks[8].y + i as u16;
        if fy >= chunks[8].y + chunks[8].height { break; }
        let label_color = if *is_foc { ACCENT } else { MUTED };
        let val_color   = if *is_foc { FG } else { FG2 };
        let display = if value.is_empty() { "—" } else { value };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("{:<LABEL_W$}", label), Style::default().fg(label_color)),
                Span::styled(display.to_owned(), Style::default().fg(val_color)),
            ])),
            Rect::new(cx, fy, cw, 1),
        );
    }

    // Status bar rendered 1 row above the bottom edge (leaves 1 row of bottom padding).
    let inner_bottom = area.y + area.height.saturating_sub(2);
    let mut show_status = |msg: &str, color| {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw(" "),
                Span::styled(msg.to_owned(), Style::default().fg(color)),
            ])),
            Rect::new(area.x, inner_bottom, area.width, 1),
        );
    };
    if detail.saving {
        show_status("Saving…", ACCENT);
    } else if let Some((rid, msg, at)) = &detail.error {
        if rid == &room.id && at.elapsed().as_secs() < 10 {
            show_status(msg, DANGER);
        }
    }

    if detail.confirm_leave {
        draw_confirm_leave(f, room.display_name.as_str());
    }
}

fn draw_members_block(f: &mut Frame, app: &App, area: Rect) {
    if area.height < 3 { return; }

    let members_focused = app.rooms_tool.detail_members_focused;
    let has_profile = app.rooms_tool.member_profile.is_some();

    // Determine title and border colour.
    let border_color = if has_profile {
        if app.rooms_tool.member_profile.as_ref().map_or(false, |p| p.confirm_ignore) {
            DANGER
        } else {
            ACCENT
        }
    } else if members_focused { ACCENT } else { BORDER };

    let (member_count, member_filtered_count, member_filter_active) =
        app.rooms_tool.members.as_ref().map_or((0, 0, false), |ms| {
            let filtered = filtered_members_vec(ms).len();
            (ms.members.len(), filtered, !ms.filter.input.is_empty())
        });

    let title = if has_profile {
        if app.rooms_tool.member_profile.as_ref().map_or(false, |p| p.confirm_ignore) {
            " Ignore User ".to_owned()
        } else {
            " Member Profile ".to_owned()
        }
    } else if member_filter_active {
        format!(" Members ({member_filtered_count}/{member_count}) ")
    } else {
        format!(" Members ({member_count}) ")
    };
    let title_color = if has_profile && app.rooms_tool.member_profile.as_ref().map_or(false, |p| p.confirm_ignore) {
        DANGER
    } else {
        border_color
    };

    let block = Block::default()
        .title(Span::styled(title, Style::default().fg(title_color)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .padding(Padding::new(1, 1, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Profile view
    if let Some(profile) = &app.rooms_tool.member_profile {
        let name = profile.display_name.as_deref().unwrap_or(&profile.user_id);
        let avatar_letter = name.chars().next().unwrap_or('?').to_ascii_uppercase();
        let pl_color = if profile.power_level >= 75 { SUCCESS }
            else if profile.power_level >= 25 { ACCENT }
            else { MUTED };

        let lines: Vec<Line> = if profile.confirm_ignore {
            vec![
                Line::from(vec![
                    Span::raw("  Ignore "),
                    Span::styled(profile.user_id.clone(), Style::default().fg(ACCENT_DIM).add_modifier(Modifier::BOLD)),
                    Span::raw("?"),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  y/Enter", Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)),
                    Span::raw("  confirm    "),
                    Span::styled("any other key", Style::default().fg(DANGER).add_modifier(Modifier::BOLD)),
                    Span::raw("  cancel"),
                ]),
            ]
        } else {
            let is_ignored = app.ignore_list.users.iter().any(|u| u == &profile.user_id);
            vec![
                Line::from(vec![
                    Span::raw(" "),
                    Span::styled(format!(" {} ", avatar_letter), Style::default().fg(BG).bg(ACCENT).add_modifier(Modifier::BOLD)),
                    Span::raw("  "),
                    Span::styled(name.to_owned(), Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
                    if profile.is_self { Span::styled("  (you)", Style::default().fg(MUTED)) } else { Span::raw("") },
                ]),
                Line::from(vec![
                    Span::raw("      "),
                    Span::styled(profile.user_id.clone(), Style::default().fg(MUTED)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(format!(" {:<17}", "POWER LEVEL"), Style::default().fg(MUTED)),
                    Span::styled(profile.power_level.to_string(), Style::default().fg(pl_color).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled(format!(" {:<17}", "IGNORED"), Style::default().fg(MUTED)),
                    Span::styled(
                        if is_ignored { "true" } else { "false" },
                        Style::default().fg(if is_ignored { DANGER } else { MUTED }),
                    ),
                ]),
            ]
        };
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }

    // Member list view
    let Some(ms) = &app.rooms_tool.members else {
        return;
    };

    if ms.loading {
        f.render_widget(
            Paragraph::new("  Loading members…")
                .style(Style::default().fg(MUTED).add_modifier(Modifier::ITALIC)),
            inner,
        );
        return;
    }

    // Build filtered member list.
    let filtered_members = filtered_members_vec(ms);
    let _total_members = ms.members.len();
    let filtered_member_count = filtered_members.len();

    // Reserve bottom rows for pl_edit and error.
    let has_pl_edit = members_focused && ms.pl_edit.is_some();
    let has_ms_error = members_focused && ms.error.is_some();
    let bottom_rows = has_pl_edit as u16 + has_ms_error as u16;
    let (list_area, prompt_area) = if bottom_rows > 0 && inner.height > bottom_rows {
        let c = Layout::vertical([Constraint::Min(1), Constraint::Length(bottom_rows)]).split(inner);
        (c[0], Some(c[1]))
    } else {
        (inner, None)
    };

    // Pre-compute column widths for alignment.
    let name_col_w: usize = filtered_members.iter()
        .map(|m| m.display_name.as_deref().unwrap_or("").chars().count()
            + if m.is_self { 6 } else { 0 }) // " (you)"
        .max()
        .unwrap_or(0)
        .min(32);
    let uid_col_w: usize = filtered_members.iter()
        .filter(|m| m.display_name.is_some())
        .map(|m| m.user_id.chars().count())
        .max()
        .unwrap_or(0)
        .min(40);

    let sel_idx = if members_focused {
        ms.selected.min(filtered_member_count.saturating_sub(1))
    } else {
        usize::MAX
    };
    let rows_y = list_area.y;
    let visible = list_area.height as usize;
    let scroll_off = if members_focused && sel_idx != usize::MAX && sel_idx + 1 > visible {
        sel_idx + 1 - visible
    } else {
        0
    };

    for (i, m) in filtered_members.iter().enumerate().skip(scroll_off).take(visible) {
        let row_y = rows_y + (i - scroll_off) as u16;
        let is_sel = members_focused && i == sel_idx;

        if is_sel {
            f.render_widget(
                Block::default().style(Style::default().bg(ratatui::style::Color::Rgb(40, 60, 80))),
                Rect::new(list_area.x, row_y, list_area.width, 1),
            );
        }

        let pl_str = format!("[{}]", m.power_level);
        let pl_w = pl_str.chars().count() as u16;
        let indicator_w = 2u16;
        let left_w = list_area.width.saturating_sub(indicator_w + 1 + pl_w);
        let pl_x = list_area.x + list_area.width.saturating_sub(pl_w);

        let ind_span = if is_sel {
            Span::styled("▌ ", Style::default().fg(ACCENT))
        } else {
            Span::raw("  ")
        };

        let name = m.display_name.as_deref().unwrap_or(&m.user_id);
        let self_suffix = if m.is_self { " (you)" } else { "" };
        let full_name = format!("{}{}", name, self_suffix);
        let name_color = if m.is_self { MUTED } else { ratatui::style::Color::White };

        let uid_part = if m.display_name.is_some() && uid_col_w > 0 {
            format!("  {:<uid_col_w$}", m.user_id)
        } else {
            String::new()
        };

        let left_line = Line::from(vec![
            ind_span,
            Span::styled(
                format!("{:<name_col_w$}", full_name),
                Style::default().fg(name_color),
            ),
            Span::styled(uid_part, Style::default().fg(MUTED)),
        ]);
        f.render_widget(
            Paragraph::new(left_line),
            Rect::new(list_area.x, row_y, indicator_w + left_w, 1),
        );

        let pl_color = if m.power_level >= 75 { SUCCESS }
            else if m.power_level >= 25 { ACCENT }
            else { MUTED };
        if pl_w <= list_area.width {
            f.render_widget(
                Paragraph::new(Span::styled(pl_str, Style::default().fg(pl_color))),
                Rect::new(pl_x, row_y, pl_w, 1),
            );
        }
    }

    // pl_edit prompt / member error
    if let Some(pa) = prompt_area {
        let mut sub = pa;
        if has_pl_edit {
            let input = ms.pl_edit.as_deref().unwrap_or("");
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(" Set power level: ", Style::default().fg(ACCENT_DIM)),
                    Span::styled(input.to_owned(), Style::default().fg(ratatui::style::Color::White)),
                    Span::styled("█", Style::default().fg(ACCENT_DIM)),
                ])),
                Rect::new(sub.x, sub.y, sub.width, 1),
            );
            sub = Rect::new(sub.x, sub.y + 1, sub.width, sub.height.saturating_sub(1));
        }
        if has_ms_error {
            if let Some(err) = ms.error.as_deref() {
                f.render_widget(
                    Paragraph::new(err)
                        .style(Style::default().fg(DANGER))
                        .alignment(Alignment::Center),
                    Rect::new(sub.x, sub.y, sub.width, 1),
                );
            }
        }
    }

    // Mod-action confirm dialog (overlay)
    if members_focused {
        if let Some((action, user_id)) = &ms.confirm {
            draw_mod_confirm(f, *action, user_id);
        }
    }

    // Member filter popup
    if members_focused && ms.filter.active {
        crate::ui::draw_filter_popup(f, &ms.filter, area);
    }
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
    if app.rooms_tool.filter.active {
        return filter_hint_spans(app.rooms_tool.filter.column, RoomInfo::filter_cols());
    }
    if app.rooms_tool.member_profile.is_some() {
        return vec![
            Span::styled("i", Style::default().fg(ACCENT)),
            Span::raw(" copy MXID  "),
            Span::styled("I", Style::default().fg(DANGER)),
            Span::raw(" ignore  "),
            Span::styled("Esc", Style::default().fg(ACCENT)),
            Span::raw(" close"),
        ];
    }
    if app.rooms_tool.detail_open && app.rooms_tool.detail_members_focused {
        if let Some(ms) = &app.rooms_tool.members {
            if ms.filter.active {
                return filter_hint_spans(ms.filter.column, MemberInfo::filter_cols());
            }
            if ms.pl_edit.is_some() {
                return vec![
                    Span::styled("Enter", Style::default().fg(SUCCESS)),
                    Span::raw(" set power level  "),
                    Span::styled("Esc", Style::default().fg(ACCENT)),
                    Span::raw(" cancel"),
                ];
            }
        }
        return vec![
            Span::styled("j/k", Style::default().fg(ACCENT)),
            Span::raw(" navigate  "),
            Span::styled("e/Enter", Style::default().fg(ACCENT)),
            Span::raw(" profile  "),
            Span::styled("d", Style::default().fg(ACCENT)),
            Span::raw(" detail  "),
            Span::styled("i", Style::default().fg(ACCENT)),
            Span::raw(" copy MXID  "),
            Span::styled("I", Style::default().fg(DANGER)),
            Span::raw(" ignore  "),
            Span::styled("p", Style::default().fg(ACCENT)),
            Span::raw(" PL  "),
            Span::styled("K", Style::default().fg(DANGER)),
            Span::raw(" kick  "),
            Span::styled("/", Style::default().fg(ACCENT)),
            Span::raw(" filter  "),
            Span::styled("Esc", Style::default().fg(ACCENT)),
            Span::raw(" back"),
        ];
    }
    if app.rooms_tool.detail.editing.is_some() {
        return vec![
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
            Span::styled("e/Enter", Style::default().fg(ACCENT)),
            Span::raw(" edit  "),
            Span::styled("i", Style::default().fg(ACCENT)),
            Span::raw(" copy ID  "),
            Span::styled("PgDn/PgUp", Style::default().fg(ACCENT)),
            Span::raw(" scroll topic  "),
            Span::styled("m", Style::default().fg(ACCENT)),
            Span::raw(" members  "),
            Span::styled("x", Style::default().fg(DANGER)),
            Span::raw(" leave  "),
            Span::styled("Esc", Style::default().fg(ACCENT)),
            Span::raw(" back"),
        ];
    }
    vec![
        Span::styled("j/k", Style::default().fg(ACCENT)),
        Span::raw(" navigate  "),
        Span::styled("d/Enter", Style::default().fg(ACCENT)),
        Span::raw(" detail  "),
        Span::styled("m", Style::default().fg(ACCENT)),
        Span::raw(" members  "),
        Span::styled("x", Style::default().fg(DANGER)),
        Span::raw(" leave  "),
        Span::styled("/", Style::default().fg(ACCENT)),
        Span::raw(" filter  "),
        Span::styled("r", Style::default().fg(ACCENT)),
        Span::raw(" refresh"),
    ]
}

