use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use tokio::sync::oneshot;

use crate::app::{ActiveTool, App};
use crate::matrix::DeviceInfo;
use crate::tools::{ACCENT, ERROR, FOCUSED, MUTED, SUCCESS};
use crate::ui::centered_rect;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum DeleteDialogState {
    Confirm,
    EnterPassword(String),
}

#[derive(Default)]
pub struct DevicesState {
    pub devices: Vec<DeviceInfo>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub delete_dialog: Option<(String, DeleteDialogState)>,
    pub load_rx: Option<oneshot::Receiver<Result<Vec<DeviceInfo>, String>>>,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn handle(app: &mut App, code: KeyCode) {
    // Delete dialog.
    if let Some((device_id, ref state)) = &app.devices.delete_dialog {
        let device_id = device_id.clone();
        match state {
            DeleteDialogState::Confirm => match code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    app.devices.delete_dialog =
                        Some((device_id, DeleteDialogState::EnterPassword(String::new())));
                }
                _ => {
                    app.devices.delete_dialog = None;
                }
            },
            DeleteDialogState::EnterPassword(ref pwd) => {
                let pwd = pwd.clone();
                match code {
                    KeyCode::Esc => {
                        app.devices.delete_dialog =
                            Some((device_id, DeleteDialogState::Confirm));
                    }
                    KeyCode::Backspace => {
                        let mut s = pwd.clone();
                        s.pop();
                        app.devices.delete_dialog =
                            Some((device_id, DeleteDialogState::EnterPassword(s)));
                    }
                    KeyCode::Char(c) if !c.is_control() => {
                        let mut s = pwd.clone();
                        s.push(c);
                        app.devices.delete_dialog =
                            Some((device_id, DeleteDialogState::EnterPassword(s)));
                    }
                    KeyCode::Enter => {
                        let password = pwd.clone();
                        app.devices.delete_dialog = None;
                        do_delete_device(app, &device_id, &password).await;
                    }
                    _ => {}
                }
            }
        }
        return;
    }

    if app.devices.loading {
        return;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
            app.active_tool = ActiveTool::Home;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if app.devices.selected + 1 < app.devices.devices.len() {
                app.devices.selected += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.devices.selected > 0 {
                app.devices.selected -= 1;
            }
        }
        KeyCode::Char('d') | KeyCode::Delete => {
            if let Some(dev) = app.devices.devices.get(app.devices.selected) {
                if dev.is_current {
                    app.devices.error = Some("Cannot delete the current device.".to_owned());
                } else {
                    let id = dev.device_id.clone();
                    app.devices.delete_dialog = Some((id, DeleteDialogState::Confirm));
                    app.devices.error = None;
                }
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            start_load(app);
        }
        _ => {}
    }
}

async fn do_delete_device(app: &mut App, device_id: &str, password: &str) {
    if let Some(client) = &app.matrix {
        match client.delete_device(device_id, password).await {
            Ok(()) => {
                app.devices.error = None;
            }
            Err(e) => {
                app.devices.error = Some(format!("Delete failed: {e}"));
            }
        }
    }
    start_load(app);
}

pub fn start_load(app: &mut App) {
    let Some(client) = app.matrix.clone() else { return; };
    app.devices.loading = true;
    app.devices.error = None;
    let (tx, rx) = oneshot::channel();
    app.devices.load_rx = Some(rx);
    tokio::spawn(async move {
        let result = client.get_devices().await.map_err(|e| e.to_string());
        let _ = tx.send(result);
    });
}

pub fn poll_load(app: &mut App) {
    let received = app
        .devices
        .load_rx
        .as_mut()
        .and_then(|rx| rx.try_recv().ok());
    if let Some(result) = received {
        app.devices.load_rx = None;
        match result {
            Ok(devices) => {
                if !devices.is_empty() && app.devices.selected >= devices.len() {
                    app.devices.selected = devices.len() - 1;
                }
                app.devices.devices = devices;
                app.devices.error = None;
            }
            Err(e) => {
                app.devices.error = Some(e);
            }
        }
        app.devices.loading = false;
    }
}

// ---------------------------------------------------------------------------
// Draw
// ---------------------------------------------------------------------------

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    if app.devices.loading {
        f.render_widget(
            Paragraph::new("Loading devices…")
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    if app.devices.devices.is_empty() {
        f.render_widget(
            Paragraph::new("No devices found. Press 'r' to refresh.")
                .style(Style::default().fg(MUTED))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = app
        .devices
        .devices
        .iter()
        .map(|d| {
            let name = d
                .display_name
                .as_deref()
                .unwrap_or("(unnamed)")
                .to_owned();
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
                        .fg(if d.is_current {
                            SUCCESS
                        } else {
                            ratatui::style::Color::White
                        })
                        .add_modifier(if d.is_current {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                current_marker,
                Span::styled(
                    format!("  {}", d.device_id),
                    Style::default().fg(MUTED),
                ),
                Span::styled(last_info, Style::default().fg(MUTED)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(Span::styled(
                    format!(" {} device(s) ", app.devices.devices.len()),
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
    state.select(Some(app.devices.selected));

    if let Some(err) = &app.devices.error {
        let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
        f.render_stateful_widget(list, chunks[0], &mut state);
        f.render_widget(
            Paragraph::new(err.as_str())
                .style(Style::default().fg(ERROR))
                .alignment(Alignment::Center),
            chunks[1],
        );
    } else {
        f.render_stateful_widget(list, area, &mut state);
    }

    if app.devices.delete_dialog.is_some() {
        draw_delete_dialog(f, app);
    }
}

fn draw_delete_dialog(f: &mut Frame, app: &App) {
    let Some((ref device_id, ref dialog_state)) = app.devices.delete_dialog else {
        return;
    };
    let dev_name = app
        .devices
        .devices
        .iter()
        .find(|d| &d.device_id == device_id)
        .and_then(|d| d.display_name.clone())
        .unwrap_or_else(|| device_id.clone());

    let area = f.area();
    let popup = centered_rect(58, 9, area);
    f.render_widget(Clear, popup);

    let lines = match dialog_state {
        DeleteDialogState::Confirm => vec![
            Line::from(""),
            Line::from(vec![
                Span::raw("  Sign out device: "),
                Span::styled(
                    dev_name,
                    Style::default().fg(FOCUSED).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "  y/Enter",
                    Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  continue    "),
                Span::styled(
                    "any other key",
                    Style::default().fg(ERROR),
                ),
                Span::raw("  cancel"),
            ]),
        ],
        DeleteDialogState::EnterPassword(pwd) => vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Enter your account password to confirm:",
                Style::default().fg(ratatui::style::Color::White),
            )]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Password: ", Style::default().fg(FOCUSED)),
                Span::styled(
                    "•".repeat(pwd.len()),
                    Style::default().fg(ratatui::style::Color::White),
                ),
                Span::styled("█", Style::default().fg(FOCUSED)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "  Enter",
                    Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
                ),
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
                        Style::default().fg(ERROR).add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ERROR))
                    .style(Style::default().bg(ratatui::style::Color::Rgb(25, 15, 15))),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}

pub fn hint_spans(app: &App) -> Vec<Span<'static>> {
    if app.devices.delete_dialog.is_some() {
        vec![]
    } else {
        vec![
            Span::styled("j/k", Style::default().fg(ACCENT)),
            Span::raw(" navigate  "),
            Span::styled("d", Style::default().fg(ERROR)),
            Span::raw(" sign out  "),
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
    "Devices"
}
