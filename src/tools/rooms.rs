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
use crate::tools::{ACCENT, ERROR, FOCUSED, MUTED, SUCCESS, FilterState};
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

impl Default for DetailField {
    fn default() -> Self {
        Self::Name
    }
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
    pub confirm: Option<(ModAction, String)>, // (action, user_id)
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

    // Detail view (Some = detail open, None = list)
    pub detail_idx: Option<usize>,
    pub detail: DetailState,

    // Member view (Some = member list open, only valid when detail_idx is Some)
    pub members: Option<MembersState>,
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
            detail_idx: None,
            detail: DetailState::default(),
            members: None,
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

    // Detail view.
    if app.rooms_tool.detail_idx.is_some() {
        handle_detail(app, code).await;
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

    // Normal list.
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
        KeyCode::Enter => open_detail(app),
        _ => {}
    }
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
        KeyCode::Enter => open_detail(app),
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

async fn handle_leave_select(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.rooms_tool.leave_select = false;
            app.rooms_tool.checked.clear();
        }
        KeyCode::Char('j') | KeyCode::Down => nav_down(app),
        KeyCode::Char('k') | KeyCode::Up => nav_up(app),
        KeyCode::Char(' ') | KeyCode::Char('x') => {
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

async fn handle_detail(app: &mut App, code: KeyCode) {
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
        return;
    }

    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.rooms_tool.detail_idx = None;
            app.rooms_tool.detail = DetailState::default();
        }
        KeyCode::Tab | KeyCode::Char('j') | KeyCode::Down => {
            app.rooms_tool.detail.focused = match app.rooms_tool.detail.focused {
                DetailField::Name => DetailField::Topic,
                DetailField::Topic => DetailField::Name,
            };
        }
        KeyCode::BackTab | KeyCode::Char('k') | KeyCode::Up => {
            app.rooms_tool.detail.focused = match app.rooms_tool.detail.focused {
                DetailField::Name => DetailField::Topic,
                DetailField::Topic => DetailField::Name,
            };
        }
        KeyCode::Char('e') | KeyCode::Enter => {
            let idx = app.rooms_tool.detail_idx.unwrap();
            if let Some(room) = app.rooms_tool.rooms.get(idx) {
                let current = match app.rooms_tool.detail.focused {
                    DetailField::Name => room.display_name.clone(),
                    DetailField::Topic => room.topic.clone().unwrap_or_default(),
                };
                app.rooms_tool.detail.editing = Some(current);
                app.rooms_tool.detail.error = None;
                app.rooms_tool.detail.success = None;
            }
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            app.rooms_tool.detail.confirm_leave = true;
        }
        KeyCode::Char('m') | KeyCode::Char('M') => {
            app.rooms_tool.members = Some(MembersState::default());
            start_member_load(app);
        }
        _ => {}
    }
}

async fn handle_members(app: &mut App, code: KeyCode) {
    let Some(ms) = &app.rooms_tool.members else { return; };

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

fn open_detail(app: &mut App) {
    let filtered = app.rooms_tool.filtered_rooms();
    if let Some(room) = filtered.get(app.rooms_tool.selected) {
        let id = room.id.clone();
        if let Some(idx) = app.rooms_tool.rooms.iter().position(|r| r.id == id) {
            app.rooms_tool.detail_idx = Some(idx);
            app.rooms_tool.detail = DetailState::default();
        }
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
    let Some(idx) = app.rooms_tool.detail_idx else { return; };
    let Some(room) = app.rooms_tool.rooms.get(idx) else { return; };
    let room_id = room.id.clone();

    if let Some(client) = &app.matrix {
        match client.leave_room(&room_id).await {
            Ok(()) => {
                app.rooms_tool.rooms.retain(|r| r.id != room_id);
                app.rooms_tool.detail_idx = None;
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
    let Some(idx) = app.rooms_tool.detail_idx else { return; };
    let Some(room) = app.rooms_tool.rooms.get(idx) else { return; };
    let room_id = room.id.clone();
    let val = app.rooms_tool.detail.editing.take().unwrap_or_default();

    app.rooms_tool.detail.saving = true;
    app.rooms_tool.detail.error = None;

    let result = if let Some(client) = &app.matrix {
        match app.rooms_tool.detail.focused {
            DetailField::Name => client.set_room_name(&room_id, val.clone()).await,
            DetailField::Topic => client.set_room_topic(&room_id, &val).await,
        }
    } else {
        Err(anyhow::anyhow!("Not connected"))
    };

    match result {
        Ok(()) => {
            // Update local cache
            if let Some(room) = app.rooms_tool.rooms.get_mut(idx) {
                match app.rooms_tool.detail.focused {
                    DetailField::Name => room.display_name = val,
                    DetailField::Topic => room.topic = if val.is_empty() { None } else { Some(val) },
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
    let Some(idx) = app.rooms_tool.detail_idx else { return; };
    let Some(room) = app.rooms_tool.rooms.get(idx) else { return; };
    let room_id = room.id.clone();
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

async fn do_mod_action(app: &mut App, action: ModAction, user_id: String) {
    let Some(idx) = app.rooms_tool.detail_idx else { return; };
    let Some(room) = app.rooms_tool.rooms.get(idx) else { return; };
    let room_id = room.id.clone();
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
    // Member view takes precedence.
    if app.rooms_tool.members.is_some() {
        draw_members(f, app, area);
        return;
    }

    // Detail view.
    if let Some(idx) = app.rooms_tool.detail_idx {
        draw_detail(f, app, area, idx);
        return;
    }

    // Leaving progress overlay.
    if !app.rooms_tool.leaving_items.is_empty() {
        draw_leaving(f, app, area);
        return;
    }

    // List.
    draw_list(f, app, area);
}

fn draw_list(f: &mut Frame, app: &App, area: Rect) {
    if app.rooms_tool.loading {
        f.render_widget(
            Paragraph::new("Syncing rooms…")
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let show_filter = app.rooms_tool.filter.active || !app.rooms_tool.filter.input.is_empty();
    let chunks = if show_filter {
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area)
    } else {
        Layout::vertical([Constraint::Min(1)]).split(area)
    };
    let (filter_area, list_area) = if show_filter {
        (Some(chunks[0]), chunks[1])
    } else {
        (None, chunks[0])
    };

    if let Some(fa) = filter_area {
        draw_filter_row(f, app, fa);
    }

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

    let items: Vec<ListItem> = filtered
        .iter()
        .map(|r| {
            let checked = if leave_select {
                if app.rooms_tool.checked.contains(&r.id) {
                    Span::styled("[✓] ", Style::default().fg(ERROR).add_modifier(Modifier::BOLD))
                } else {
                    Span::styled("[ ] ", Style::default().fg(MUTED))
                }
            } else {
                Span::raw("")
            };
            let member_text = if r.member_count > 0 {
                format!("  {} members", r.member_count)
            } else {
                String::new()
            };
            let alias_text = r.alias.as_deref().map(|a| format!("  {a}")).unwrap_or_default();
            ListItem::new(Line::from(vec![
                checked,
                Span::styled(r.display_name.clone(), Style::default().fg(ratatui::style::Color::White)),
                Span::styled(member_text, Style::default().fg(MUTED)),
                Span::styled(alias_text, Style::default().fg(MUTED)),
            ]))
        })
        .collect();

    let title = if leave_select {
        format!(
            " {} room(s) — {} selected — Enter to leave ",
            app.rooms_tool.rooms.len(),
            app.rooms_tool.checked.len()
        )
    } else {
        format!(" {} room(s) ", app.rooms_tool.rooms.len())
    };

    let border_color = if leave_select { ERROR } else { ACCENT };

    let list = List::new(items)
        .block(
            Block::default()
                .title(Span::styled(title, Style::default().fg(border_color)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color)),
        )
        .highlight_style(
            Style::default()
                .bg(ratatui::style::Color::Rgb(40, 60, 80))
                .fg(FOCUSED)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    state.select(if filtered.is_empty() {
        None
    } else {
        Some(app.rooms_tool.selected.min(filtered.len() - 1))
    });

    if let Some(err) = &app.rooms_tool.error {
        let ec = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(list_area);
        f.render_stateful_widget(list, ec[0], &mut state);
        f.render_widget(
            Paragraph::new(err.as_str())
                .style(Style::default().fg(ERROR))
                .alignment(Alignment::Center),
            ec[1],
        );
    } else {
        f.render_stateful_widget(list, list_area, &mut state);
    }
}

fn draw_filter_row(f: &mut Frame, app: &App, area: Rect) {
    let filter = &app.rooms_tool.filter;
    let line = if filter.active {
        Line::from(vec![
            Span::styled(" Filter: ", Style::default().fg(FOCUSED)),
            Span::styled(filter.input.clone(), Style::default().fg(ratatui::style::Color::White)),
            Span::styled("█", Style::default().fg(FOCUSED)),
            Span::styled("  Esc to clear", Style::default().fg(MUTED)),
        ])
    } else {
        Line::from(vec![Span::styled(
            format!(" Filter: {}  / to search", filter.input),
            Style::default().fg(MUTED),
        )])
    };
    f.render_widget(Paragraph::new(line), area);
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
                    Span::styled("  ✗ ", Style::default().fg(ERROR).add_modifier(Modifier::BOLD)),
                    ERROR,
                ),
            };
            let mut spans = vec![
                status_span,
                Span::styled(item.room_name.clone(), Style::default().fg(name_color)),
            ];
            if let LeaveStatus::Failed(ref e) = item.status {
                spans.push(Span::styled(
                    format!("  {e}"),
                    Style::default().fg(ERROR),
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

fn draw_detail(f: &mut Frame, app: &App, area: Rect, idx: usize) {
    let Some(room) = app.rooms_tool.rooms.get(idx) else { return; };
    let detail = &app.rooms_tool.detail;

    let name_text = detail
        .editing
        .as_deref()
        .filter(|_| detail.focused == DetailField::Name)
        .map(|s| s.to_owned())
        .unwrap_or_else(|| room.display_name.clone());

    let topic_text = detail
        .editing
        .as_deref()
        .filter(|_| detail.focused == DetailField::Topic)
        .map(|s| s.to_owned())
        .unwrap_or_else(|| room.topic.clone().unwrap_or_else(|| "(none)".to_owned()));

    let make_field = |label: &str, value: &str, focused: bool, editing: bool| -> Paragraph<'static> {
        let border_color = if editing { FOCUSED } else if focused { ACCENT } else { MUTED };
        let display = if editing { format!("{value}█") } else { value.to_owned() };
        Paragraph::new(display).block(
            Block::default()
                .title(Span::styled(
                    format!(" {label} "),
                    Style::default().fg(border_color),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color)),
        )
    };

    let name_editing = detail.editing.is_some() && detail.focused == DetailField::Name;
    let topic_editing = detail.editing.is_some() && detail.focused == DetailField::Topic;
    let name_focused = detail.focused == DetailField::Name;
    let topic_focused = detail.focused == DetailField::Topic;

    let chunks = Layout::vertical([
        Constraint::Length(1),  // padding
        Constraint::Length(3),  // name
        Constraint::Length(1),  // gap
        Constraint::Length(3),  // topic
        Constraint::Length(1),  // gap
        Constraint::Length(5),  // info block
        Constraint::Length(1),  // status/error
        Constraint::Min(0),     // hints
    ])
    .split(area);

    f.render_widget(make_field("Name", &name_text, name_focused, name_editing), chunks[1]);
    f.render_widget(make_field("Topic", &topic_text, topic_focused, topic_editing), chunks[3]);

    // Info block.
    let info_lines = vec![
        Line::from(vec![
            Span::styled(format!("{:<14}", "Room ID"), Style::default().fg(MUTED)),
            Span::styled(room.id.clone(), Style::default().fg(ratatui::style::Color::White)),
        ]),
        Line::from(vec![
            Span::styled(format!("{:<14}", "Alias"), Style::default().fg(MUTED)),
            Span::styled(
                room.alias.clone().unwrap_or_else(|| "(none)".to_owned()),
                Style::default().fg(ratatui::style::Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(format!("{:<14}", "Members"), Style::default().fg(MUTED)),
            Span::styled(
                room.member_count.to_string(),
                Style::default().fg(ratatui::style::Color::White),
            ),
        ]),
    ];
    f.render_widget(
        Paragraph::new(info_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(MUTED)),
        ),
        chunks[5],
    );

    // Status line.
    let status: Paragraph = if detail.saving {
        Paragraph::new("Saving…")
            .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
            .alignment(Alignment::Center)
    } else if let Some(err) = &detail.error {
        Paragraph::new(err.as_str())
            .style(Style::default().fg(ERROR))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
    } else if let Some(ok) = &detail.success {
        Paragraph::new(ok.as_str())
            .style(Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD))
            .alignment(Alignment::Center)
    } else {
        Paragraph::new("")
    };
    f.render_widget(status, chunks[6]);

    let hint = if detail.editing.is_some() {
        "Enter save  •  Esc discard"
    } else {
        "Tab/j/k field  •  e/Enter edit  •  m members  •  d leave  •  Esc/q back"
    };
    f.render_widget(
        Paragraph::new(hint)
            .style(Style::default().fg(MUTED))
            .alignment(Alignment::Center),
        chunks[7],
    );

    if detail.confirm_leave {
        draw_confirm_leave(f, room.display_name.as_str());
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
                Span::styled(room_name.to_owned(), Style::default().fg(FOCUSED).add_modifier(Modifier::BOLD)),
                Span::raw("?"),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  y/Enter", Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)),
                Span::raw("  confirm    "),
                Span::styled("any other key", Style::default().fg(ERROR).add_modifier(Modifier::BOLD)),
                Span::raw("  cancel"),
            ]),
        ])
        .block(
            Block::default()
                .title(Span::styled(" Confirm ", Style::default().fg(ERROR).add_modifier(Modifier::BOLD)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ERROR))
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

    if ms.members.is_empty() {
        f.render_widget(
            Paragraph::new("No members found.")
                .style(Style::default().fg(MUTED))
                .alignment(Alignment::Center),
            area,
        );
    } else {
        let items: Vec<ListItem> = ms
            .members
            .iter()
            .map(|m| {
                let name = m.display_name.as_deref().unwrap_or(&m.user_id).to_owned();
                let uid = if m.display_name.is_some() {
                    format!("  {}", m.user_id)
                } else {
                    String::new()
                };
                let pl = if m.power_level >= 75 {
                    Span::styled(" ★", Style::default().fg(SUCCESS))
                } else if m.power_level >= 25 {
                    Span::styled(" ◆", Style::default().fg(ACCENT))
                } else {
                    Span::raw("")
                };
                ListItem::new(Line::from(vec![
                    Span::styled(name, Style::default().fg(ratatui::style::Color::White)),
                    pl,
                    Span::styled(uid, Style::default().fg(MUTED)),
                ]))
            })
            .collect();

        let room_name = app
            .rooms_tool
            .detail_idx
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
                    .fg(FOCUSED)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");

        let mut state = ListState::default();
        state.select(if ms.members.is_empty() { None } else { Some(ms.selected) });

        if let Some(err) = &ms.error {
            let ec =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
            f.render_stateful_widget(list, ec[0], &mut state);
            f.render_widget(
                Paragraph::new(err.as_str())
                    .style(Style::default().fg(ERROR))
                    .alignment(Alignment::Center),
                ec[1],
            );
        } else {
            f.render_stateful_widget(list, area, &mut state);
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
                Span::styled(user_id.to_owned(), Style::default().fg(FOCUSED).add_modifier(Modifier::BOLD)),
                Span::raw("?"),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  y/Enter", Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)),
                Span::raw("  confirm    "),
                Span::styled("any other key", Style::default().fg(ERROR).add_modifier(Modifier::BOLD)),
                Span::raw("  cancel"),
            ]),
        ])
        .block(
            Block::default()
                .title(Span::styled(format!(" {verb} Member "), Style::default().fg(ERROR).add_modifier(Modifier::BOLD)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ERROR))
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
    if app.rooms_tool.members.is_some() {
        return vec![
            Span::styled("j/k", Style::default().fg(ACCENT)),
            Span::raw(" navigate  "),
            Span::styled("K", Style::default().fg(ERROR)),
            Span::raw(" kick  "),
            Span::styled("b", Style::default().fg(ERROR)),
            Span::raw(" ban  "),
            Span::styled("r", Style::default().fg(ACCENT)),
            Span::raw(" refresh  "),
            Span::styled("Esc/q", Style::default().fg(ACCENT)),
            Span::raw(" back"),
        ];
    }
    if app.rooms_tool.detail_idx.is_some() {
        let detail = &app.rooms_tool.detail;
        return if detail.editing.is_some() {
            vec![
                Span::styled("Enter", Style::default().fg(SUCCESS)),
                Span::raw(" save  "),
                Span::styled("Esc", Style::default().fg(ACCENT)),
                Span::raw(" discard"),
            ]
        } else {
            vec![
                Span::styled("Tab/j/k", Style::default().fg(ACCENT)),
                Span::raw(" field  "),
                Span::styled("e/Enter", Style::default().fg(ACCENT)),
                Span::raw(" edit  "),
                Span::styled("m", Style::default().fg(ACCENT)),
                Span::raw(" members  "),
                Span::styled("d", Style::default().fg(ERROR)),
                Span::raw(" leave  "),
                Span::styled("Esc/q", Style::default().fg(ACCENT)),
                Span::raw(" back"),
            ]
        };
    }
    if app.rooms_tool.leave_select {
        return vec![
            Span::styled("j/k", Style::default().fg(ACCENT)),
            Span::raw(" navigate  "),
            Span::styled("Space/x", Style::default().fg(ACCENT)),
            Span::raw(" select  "),
            Span::styled("Enter", Style::default().fg(ERROR)),
            Span::raw(" leave selected  "),
            Span::styled("Esc", Style::default().fg(ACCENT)),
            Span::raw(" cancel"),
        ];
    }
    vec![
        Span::styled("j/k", Style::default().fg(ACCENT)),
        Span::raw(" navigate  "),
        Span::styled("Enter", Style::default().fg(ACCENT)),
        Span::raw(" detail  "),
        Span::styled("d", Style::default().fg(ERROR)),
        Span::raw(" leave rooms  "),
        Span::styled("/", Style::default().fg(ACCENT)),
        Span::raw(" filter  "),
        Span::styled("r", Style::default().fg(ACCENT)),
        Span::raw(" refresh  "),
        Span::styled(":", Style::default().fg(ACCENT)),
        Span::raw(" command  "),
        Span::styled("Esc/q", Style::default().fg(ACCENT)),
        Span::raw(" home"),
    ]
}

pub fn tool_name() -> &'static str {
    "Rooms"
}
