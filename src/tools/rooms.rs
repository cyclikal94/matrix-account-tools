use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use crate::app::{ActiveTool, App};
use crate::matrix::RoomInfo;
use crate::tools::{ACCENT, ERROR, FOCUSED, MUTED, FilterState};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct RoomBrowserState {
    pub rooms: Vec<RoomInfo>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub filter: FilterState,
    pub detail: Option<usize>, // index into rooms when in detail view
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
// Handler
// ---------------------------------------------------------------------------

pub async fn handle(app: &mut App, code: KeyCode) {
    // Detail sub-screen.
    if app.rooms_tool.detail.is_some() {
        if matches!(code, KeyCode::Esc | KeyCode::Char('q')) {
            app.rooms_tool.detail = None;
        }
        return;
    }

    // Filter input mode.
    if app.rooms_tool.filter.active {
        match code {
            KeyCode::Esc => app.rooms_tool.filter.clear(),
            KeyCode::Backspace => {
                app.rooms_tool.filter.input.pop();
            }
            KeyCode::Char(c) if !c.is_control() => {
                app.rooms_tool.filter.input.push(c);
                app.rooms_tool.selected = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = app.rooms_tool.filtered_rooms().len();
                if app.rooms_tool.selected + 1 < len {
                    app.rooms_tool.selected += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if app.rooms_tool.selected > 0 {
                    app.rooms_tool.selected -= 1;
                }
            }
            KeyCode::Enter => open_detail(app),
            _ => {}
        }
        return;
    }

    if app.rooms_tool.loading {
        return;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            app.active_tool = ActiveTool::Home;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let len = app.rooms_tool.filtered_rooms().len();
            if app.rooms_tool.selected + 1 < len {
                app.rooms_tool.selected += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.rooms_tool.selected > 0 {
                app.rooms_tool.selected -= 1;
            }
        }
        KeyCode::Char('/') => {
            app.rooms_tool.filter.active = true;
            app.rooms_tool.filter.input.clear();
            app.rooms_tool.selected = 0;
        }
        KeyCode::Enter => open_detail(app),
        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.rooms_tool.loading = true;
            do_load_rooms(app).await;
        }
        _ => {}
    }
}

fn open_detail(app: &mut App) {
    let filtered = app.rooms_tool.filtered_rooms();
    if let Some(room) = filtered.get(app.rooms_tool.selected) {
        let id = room.id.clone();
        // Find the index in the full rooms list.
        let idx = app.rooms_tool.rooms.iter().position(|r| r.id == id);
        app.rooms_tool.detail = idx;
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
}

// ---------------------------------------------------------------------------
// Draw
// ---------------------------------------------------------------------------

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    if let Some(idx) = app.rooms_tool.detail {
        draw_detail(f, app, area, idx);
        return;
    }

    if app.rooms_tool.loading {
        f.render_widget(
            Paragraph::new("Syncing with server…")
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let show_filter =
        app.rooms_tool.filter.active || !app.rooms_tool.filter.input.is_empty();

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
        draw_filter_row(f, &app.rooms_tool.filter, fa);
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

    let items: Vec<ListItem> = filtered
        .iter()
        .map(|r| {
            let member_text = if r.member_count > 0 {
                format!("  {} members", r.member_count)
            } else {
                String::new()
            };
            let alias_text = r
                .alias
                .as_deref()
                .map(|a| format!("  {a}"))
                .unwrap_or_default();
            ListItem::new(Line::from(vec![
                Span::styled(
                    r.display_name.clone(),
                    Style::default().fg(ratatui::style::Color::White),
                ),
                Span::styled(member_text, Style::default().fg(MUTED)),
                Span::styled(alias_text, Style::default().fg(MUTED)),
            ]))
        })
        .collect();

    let title = format!(" {} room(s) ", app.rooms_tool.rooms.len());

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

fn draw_filter_row(f: &mut Frame, filter: &FilterState, area: Rect) {
    let line = if filter.active {
        Line::from(vec![
            Span::styled(" Filter: ", Style::default().fg(FOCUSED)),
            Span::styled(
                filter.input.clone(),
                Style::default().fg(ratatui::style::Color::White),
            ),
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

fn draw_detail(f: &mut Frame, app: &App, area: Rect, idx: usize) {
    let Some(room) = app.rooms_tool.rooms.get(idx) else {
        return;
    };

    let field = |label: &'static str, value: Option<&str>| -> Line<'static> {
        let val = value.unwrap_or("(none)").to_owned();
        Line::from(vec![
            Span::styled(format!("{label:<18}"), Style::default().fg(MUTED)),
            Span::styled(val, Style::default().fg(ratatui::style::Color::White)),
        ])
    };

    let lines = vec![
        Line::from(""),
        field("Name", Some(&room.display_name)),
        field("Room ID", Some(&room.id)),
        field(
            "Alias",
            room.alias.as_deref(),
        ),
        field(
            "Topic",
            room.topic.as_deref().map(|t| t.lines().next().unwrap_or(t)),
        ),
        field(
            "Members",
            Some(&room.member_count.to_string()),
        ),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Esc / q  back to list",
            Style::default().fg(MUTED),
        )]),
    ];

    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(Span::styled(" Room Detail ", Style::default().fg(ACCENT)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ACCENT)),
        ),
        area,
    );
}

pub fn hint_spans(app: &App) -> Vec<Span<'static>> {
    if app.rooms_tool.detail.is_some() {
        vec![
            Span::styled("Esc/q", Style::default().fg(ACCENT)),
            Span::raw(" back to list"),
        ]
    } else if app.rooms_tool.filter.active {
        vec![
            Span::styled("Type", Style::default().fg(FOCUSED)),
            Span::raw(" to filter  "),
            Span::styled("Esc", Style::default().fg(ACCENT)),
            Span::raw(" clear  "),
            Span::styled("Enter", Style::default().fg(ACCENT)),
            Span::raw(" open detail"),
        ]
    } else {
        vec![
            Span::styled("j/k", Style::default().fg(ACCENT)),
            Span::raw(" navigate  "),
            Span::styled("Enter", Style::default().fg(ACCENT)),
            Span::raw(" detail  "),
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
    "Rooms"
}
