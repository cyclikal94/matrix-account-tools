use std::collections::HashSet;

use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use tokio::sync::mpsc;

use crate::app::{ActiveTool, App};
use crate::matrix::RoomInfo;
use crate::tools::{ACCENT, ERROR, FOCUSED, MUTED, SUCCESS, FilterState};
use crate::ui::centered_rect;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaveStatus {
    Pending,
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

pub struct LeaveRoomsState {
    pub rooms: Vec<RoomInfo>,
    pub selected: usize,
    pub checked: HashSet<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub filter: FilterState,
    pub confirm_open: bool,
    pub leaving_items: Vec<LeaveItem>,
    pub leave_rx: Option<mpsc::Receiver<(String, Result<(), String>)>>,
}

impl Default for LeaveRoomsState {
    fn default() -> Self {
        Self {
            rooms: Vec::new(),
            selected: 0,
            checked: HashSet::new(),
            loading: false,
            error: None,
            filter: FilterState::default(),
            confirm_open: false,
            leaving_items: Vec::new(),
            leave_rx: None,
        }
    }
}

impl LeaveRoomsState {
    pub fn filtered_rooms(&self) -> Vec<&RoomInfo> {
        self.rooms
            .iter()
            .filter(|r| self.filter.matches(&r.display_name))
            .collect()
    }

    pub fn is_leaving(&self) -> bool {
        !self.leaving_items.is_empty()
    }

    pub fn leaving_complete(&self) -> bool {
        !self.leaving_items.is_empty()
            && self
                .leaving_items
                .iter()
                .all(|i| matches!(i.status, LeaveStatus::Done | LeaveStatus::Failed(_)))
    }

    pub fn toggle_checked(&mut self) {
        let filtered = self
            .rooms
            .iter()
            .filter(|r| self.filter.matches(&r.display_name))
            .collect::<Vec<_>>();
        if let Some(room) = filtered.get(self.selected) {
            let id = room.id.clone();
            if self.checked.contains(&id) {
                self.checked.remove(&id);
            } else {
                self.checked.insert(id);
            }
        }
    }

    pub fn checked_rooms(&self) -> Vec<&RoomInfo> {
        self.rooms
            .iter()
            .filter(|r| self.checked.contains(&r.id))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn handle(app: &mut App, code: KeyCode) {
    // After leaving completes, any key dismisses the progress view.
    if app.leave_rooms.leaving_complete() {
        let left_ids: HashSet<String> = app
            .leave_rooms
            .leaving_items
            .iter()
            .filter(|i| i.status == LeaveStatus::Done)
            .map(|i| i.room_id.clone())
            .collect();
        app.leave_rooms.rooms.retain(|r| !left_ids.contains(&r.id));
        app.leave_rooms.checked.retain(|id| !left_ids.contains(id));
        if !app.leave_rooms.rooms.is_empty()
            && app.leave_rooms.selected >= app.leave_rooms.rooms.len()
        {
            app.leave_rooms.selected = app.leave_rooms.rooms.len() - 1;
        }
        app.leave_rooms.leaving_items.clear();
        app.leave_rooms.leave_rx = None;
        return;
    }

    if app.leave_rooms.is_leaving() {
        return;
    }

    if app.leave_rooms.confirm_open {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => start_leaving(app),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.leave_rooms.confirm_open = false;
            }
            _ => {}
        }
        return;
    }

    // Filter input mode.
    if app.leave_rooms.filter.active {
        match code {
            KeyCode::Esc => app.leave_rooms.filter.clear(),
            KeyCode::Backspace => {
                app.leave_rooms.filter.input.pop();
            }
            KeyCode::Char(c) if !c.is_control() => {
                app.leave_rooms.filter.input.push(c);
                app.leave_rooms.selected = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = app.leave_rooms.filtered_rooms().len();
                if app.leave_rooms.selected + 1 < len {
                    app.leave_rooms.selected += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if app.leave_rooms.selected > 0 {
                    app.leave_rooms.selected -= 1;
                }
            }
            KeyCode::Char(' ') => app.leave_rooms.toggle_checked(),
            KeyCode::Enter => {
                if !app.leave_rooms.checked.is_empty() {
                    app.leave_rooms.confirm_open = true;
                }
            }
            _ => {}
        }
        return;
    }

    if app.leave_rooms.loading {
        return;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
            app.active_tool = ActiveTool::Home;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let len = app.leave_rooms.filtered_rooms().len();
            if app.leave_rooms.selected + 1 < len {
                app.leave_rooms.selected += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.leave_rooms.selected > 0 {
                app.leave_rooms.selected -= 1;
            }
        }
        KeyCode::Char('/') => {
            app.leave_rooms.filter.active = true;
            app.leave_rooms.filter.input.clear();
            app.leave_rooms.selected = 0;
        }
        KeyCode::Char(' ') => app.leave_rooms.toggle_checked(),
        KeyCode::Enter => {
            if !app.leave_rooms.checked.is_empty() {
                app.leave_rooms.confirm_open = true;
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.leave_rooms.loading = true;
            do_load_rooms(app).await;
        }
        _ => {}
    }
}

fn start_leaving(app: &mut App) {
    let items: Vec<LeaveItem> = app
        .leave_rooms
        .rooms
        .iter()
        .filter(|r| app.leave_rooms.checked.contains(&r.id))
        .map(|r| LeaveItem {
            room_id: r.id.clone(),
            room_name: r.display_name.clone(),
            status: LeaveStatus::Pending,
        })
        .collect();

    app.leave_rooms.confirm_open = false;
    if items.is_empty() {
        return;
    }

    let n = items.len().max(1);
    let (tx, rx) = mpsc::channel::<(String, Result<(), String>)>(n);
    app.leave_rooms.leave_rx = Some(rx);

    if let Some(client) = app.matrix.clone() {
        for item in &items {
            let tx = tx.clone();
            let room_id = item.room_id.clone();
            let c = client.clone();
            tokio::spawn(async move {
                let result = c
                    .leave_room(&room_id)
                    .await
                    .map_err(|e| e.to_string());
                let _ = tx.send((room_id, result)).await;
            });
        }
    }

    app.leave_rooms.leaving_items = items;
    // Set all to InProgress since we're leaving in parallel.
    for item in &mut app.leave_rooms.leaving_items {
        item.status = LeaveStatus::InProgress;
    }
}

pub async fn do_load_rooms(app: &mut App) {
    if let Some(client) = &app.matrix {
        match client.get_joined_rooms().await {
            Ok(rooms) => {
                let valid: HashSet<String> = rooms.iter().map(|r| r.id.clone()).collect();
                app.leave_rooms.checked.retain(|id| valid.contains(id));
                app.leave_rooms.rooms = rooms;
                app.leave_rooms.error = None;
                let filtered_len = app.leave_rooms.filtered_rooms().len();
                if !filtered_len == 0 || app.leave_rooms.selected >= filtered_len {
                    app.leave_rooms.selected = 0;
                }
            }
            Err(e) => {
                app.leave_rooms.error = Some(format!("{e}"));
            }
        }
    }
    app.leave_rooms.loading = false;
}

/// Drain pending results from the parallel leave channel. Call each render cycle.
pub fn poll_leave_results(app: &mut App) {
    use tokio::sync::mpsc::error::TryRecvError;

    let Some(rx) = &mut app.leave_rooms.leave_rx else {
        return;
    };

    loop {
        match rx.try_recv() {
            Ok((room_id, Ok(()))) => {
                if let Some(item) = app
                    .leave_rooms
                    .leaving_items
                    .iter_mut()
                    .find(|i| i.room_id == room_id)
                {
                    item.status = LeaveStatus::Done;
                }
            }
            Ok((room_id, Err(msg))) => {
                if let Some(item) = app
                    .leave_rooms
                    .leaving_items
                    .iter_mut()
                    .find(|i| i.room_id == room_id)
                {
                    item.status = LeaveStatus::Failed(msg);
                }
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                app.leave_rooms.leave_rx = None;
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Draw
// ---------------------------------------------------------------------------

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    if app.leave_rooms.is_leaving() {
        draw_progress(f, app, area);
        return;
    }

    if app.leave_rooms.loading {
        f.render_widget(
            Paragraph::new("Syncing with server…")
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    if app.leave_rooms.rooms.is_empty() {
        f.render_widget(
            Paragraph::new("No joined rooms found.")
                .style(Style::default().fg(MUTED))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    // Split area: optional filter row + list.
    let show_filter = app.leave_rooms.filter.active
        || !app.leave_rooms.filter.input.is_empty();
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

    // Always show the filter hint row at the top.
    if let Some(fa) = filter_area {
        draw_filter_row(f, &app.leave_rooms.filter, fa);
    }

    let filtered = app.leave_rooms.filtered_rooms();
    let checked_count = app.leave_rooms.checked.len();

    let title = if checked_count > 0 {
        format!(
            " {} room(s)  •  {} selected ",
            app.leave_rooms.rooms.len(),
            checked_count
        )
    } else {
        format!(" {} room(s) ", app.leave_rooms.rooms.len())
    };

    let items: Vec<ListItem> = filtered
        .iter()
        .map(|r| {
            let is_checked = app.leave_rooms.checked.contains(&r.id);
            let checkbox = if is_checked {
                Span::styled("[✓] ", Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("[ ] ", Style::default().fg(MUTED))
            };
            let name_style = if is_checked {
                Style::default()
                    .fg(ratatui::style::Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(ratatui::style::Color::White)
            };
            let member_text = if r.member_count > 0 {
                format!(" ({} members)", r.member_count)
            } else {
                String::new()
            };
            ListItem::new(Line::from(vec![
                checkbox,
                Span::styled(r.display_name.clone(), name_style),
                Span::styled(member_text, Style::default().fg(MUTED)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(Span::styled(title, Style::default().fg(ACCENT)))
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
    state.select(if filtered.is_empty() {
        None
    } else {
        Some(app.leave_rooms.selected.min(filtered.len() - 1))
    });

    if let Some(err) = &app.leave_rooms.error {
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

    if app.leave_rooms.confirm_open {
        draw_confirm_overlay(f, app);
    }
}

fn draw_filter_row(f: &mut Frame, filter: &FilterState, area: Rect) {
    let line = if filter.active {
        Line::from(vec![
            Span::styled(" Filter: ", Style::default().fg(FOCUSED)),
            Span::styled(filter.input.clone(), Style::default().fg(ratatui::style::Color::White)),
            Span::styled("█", Style::default().fg(FOCUSED)),
            Span::styled("  Esc to clear", Style::default().fg(MUTED)),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                format!(" Filter: {}  / to search", filter.input),
                Style::default().fg(MUTED),
            ),
        ])
    };
    f.render_widget(Paragraph::new(line), area);
}

fn draw_progress(f: &mut Frame, app: &App, area: Rect) {
    let bar_width = (area.width.saturating_sub(12) as usize).min(28).max(10);
    let name_width = (area.width.saturating_sub(bar_width as u16 + 14)) as usize;

    let done = app
        .leave_rooms
        .leaving_items
        .iter()
        .filter(|i| i.status == LeaveStatus::Done)
        .count();
    let failed = app
        .leave_rooms
        .leaving_items
        .iter()
        .filter(|i| matches!(i.status, LeaveStatus::Failed(_)))
        .count();
    let total = app.leave_rooms.leaving_items.len();

    let items: Vec<ListItem> = app
        .leave_rooms
        .leaving_items
        .iter()
        .map(|item| {
            let (icon, icon_style, filled, bar_color) = match &item.status {
                LeaveStatus::Pending => ("[ ]", Style::default().fg(MUTED), 0, MUTED),
                LeaveStatus::InProgress => (
                    "[⟳]",
                    Style::default().fg(FOCUSED).add_modifier(Modifier::BOLD),
                    bar_width / 2,
                    FOCUSED,
                ),
                LeaveStatus::Done => (
                    "[✓]",
                    Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
                    bar_width,
                    SUCCESS,
                ),
                LeaveStatus::Failed(_) => (
                    "[✗]",
                    Style::default().fg(ERROR).add_modifier(Modifier::BOLD),
                    bar_width,
                    ERROR,
                ),
            };
            let name = if item.room_name.chars().count() > name_width {
                format!(
                    "{}…",
                    item.room_name
                        .chars()
                        .take(name_width.saturating_sub(1))
                        .collect::<String>()
                )
            } else {
                format!("{:<width$}", item.room_name, width = name_width)
            };
            let bar = format!(
                "{}{}",
                "█".repeat(filled),
                "░".repeat(bar_width.saturating_sub(filled))
            );
            let mut spans = vec![
                Span::styled(format!("{icon} "), icon_style),
                Span::styled(name, Style::default().fg(ratatui::style::Color::White)),
                Span::raw("  "),
                Span::styled(bar, Style::default().fg(bar_color)),
            ];
            if let LeaveStatus::Failed(msg) = &item.status {
                spans.push(Span::styled(
                    format!(" {}", msg.chars().take(20).collect::<String>()),
                    Style::default().fg(ERROR),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let complete = app.leave_rooms.leaving_complete();
    let title = if complete {
        format!(" Done: {done} left, {failed} failed — press any key to return ")
    } else {
        format!(" Leaving rooms… {done}/{total} done ")
    };
    let title_style = if complete {
        Style::default().fg(if failed > 0 { ERROR } else { SUCCESS })
    } else {
        Style::default().fg(ACCENT)
    };

    f.render_widget(
        List::new(items).block(
            Block::default()
                .title(Span::styled(title, title_style))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ACCENT)),
        ),
        area,
    );
}

fn draw_confirm_overlay(f: &mut Frame, app: &App) {
    let area = f.area();
    let checked = app.leave_rooms.checked_rooms();
    let n = checked.len();
    let height = (4 + n as u16 + 2).min(area.height.saturating_sub(4));
    let width = 56u16.min(area.width.saturating_sub(4));
    let popup = centered_rect(width, height, area);

    f.render_widget(Clear, popup);

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Leave "),
            Span::styled(
                format!("{n} room(s)"),
                Style::default().fg(FOCUSED).add_modifier(Modifier::BOLD),
            ),
            Span::raw(":"),
        ]),
    ];
    for room in &checked {
        lines.push(Line::from(vec![
            Span::styled("    • ", Style::default().fg(MUTED)),
            Span::styled(
                room.display_name.clone(),
                Style::default().fg(ratatui::style::Color::White),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            "  Enter/y",
            Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  confirm    "),
        Span::styled(
            "Esc/n",
            Style::default().fg(ERROR).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  cancel"),
    ]));

    f.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(Span::styled(
                        " Confirm ",
                        Style::default().fg(ERROR).add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ERROR))
                    .style(
                        Style::default().bg(ratatui::style::Color::Rgb(25, 15, 15)),
                    ),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}

pub fn hint_spans(app: &App) -> Vec<Span<'static>> {
    let n = app.leave_rooms.checked.len();
    if app.leave_rooms.filter.active {
        vec![
            Span::styled("Type", Style::default().fg(FOCUSED)),
            Span::raw(" to filter  "),
            Span::styled("Esc", Style::default().fg(ACCENT)),
            Span::raw(" clear  "),
            Span::styled("Space", Style::default().fg(ACCENT)),
            Span::raw(" select  "),
            Span::styled("Enter", Style::default().fg(ACCENT)),
            Span::raw(" confirm"),
        ]
    } else if n > 0 {
        vec![
            Span::styled("Space", Style::default().fg(ACCENT)),
            Span::raw(" toggle  "),
            Span::styled("Enter", Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("  Leave {n} room(s)  "),
                Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
            ),
            Span::styled("r", Style::default().fg(ACCENT)),
            Span::raw(" refresh  "),
            Span::styled(":", Style::default().fg(ACCENT)),
            Span::raw(" command"),
        ]
    } else {
        vec![
            Span::styled("j/k", Style::default().fg(ACCENT)),
            Span::raw(" navigate  "),
            Span::styled("Space", Style::default().fg(ACCENT)),
            Span::raw(" select  "),
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
}

pub fn tool_name() -> &'static str {
    "LeaveRooms"
}
