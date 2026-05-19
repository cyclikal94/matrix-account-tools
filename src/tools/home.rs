use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use crate::app::App;
use crate::tools::{ACCENT, FOCUSED, MUTED};

#[derive(Debug, Default)]
pub struct HomeState {
    pub selected: usize,
}

pub async fn handle(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('j') | KeyCode::Down => {
            if app.home.selected + 1 < crate::app::HOME_TOOLS.len() {
                app.home.selected += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.home.selected > 0 {
                app.home.selected -= 1;
            }
        }
        KeyCode::Enter => {
            let cmd = crate::app::HOME_TOOLS
                .get(app.home.selected)
                .map(|(_, cmd)| *cmd)
                .unwrap_or("");
            if !cmd.is_empty() {
                crate::app::execute_command(app, cmd).await;
            }
        }
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            app.screen = crate::app::Screen::Quitting;
        }
        _ => {}
    }
}

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let n = crate::app::HOME_TOOLS.len() as u16;
    let box_h = n + 4;

    let inner = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(box_h),
        Constraint::Min(0),
    ])
    .split(area)[1];

    let centered = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(44),
        Constraint::Min(0),
    ])
    .split(inner)[1];

    let items: Vec<ListItem> = crate::app::HOME_TOOLS
        .iter()
        .map(|(name, cmd)| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  {name:<22}"),
                    Style::default().fg(ratatui::style::Color::White),
                ),
                Span::styled(format!(":{cmd}"), Style::default().fg(MUTED)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(Span::styled(" Tools ", Style::default().fg(ACCENT)))
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
    state.select(Some(app.home.selected));
    f.render_stateful_widget(list, centered, &mut state);
}

pub fn hint_spans() -> Vec<Span<'static>> {
    vec![
        Span::styled("j/k", Style::default().fg(ACCENT)),
        Span::raw(" navigate  "),
        Span::styled("Enter", Style::default().fg(ACCENT)),
        Span::raw(" select  "),
        Span::styled(":", Style::default().fg(ACCENT)),
        Span::raw(" command  "),
        Span::styled("q", Style::default().fg(ACCENT)),
        Span::raw(" quit"),
    ]
}

pub fn tool_name() -> &'static str {
    "Home"
}
