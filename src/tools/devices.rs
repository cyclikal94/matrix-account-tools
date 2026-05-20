use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, ListItem, Paragraph, Wrap},
};
use tokio::sync::oneshot;

use crate::app::{ActiveTool, App};
use crate::matrix::DeviceInfo;
use crate::tools::{ACCENT, ACCENT_DIM, DANGER, MUTED, SUCCESS, FilterState, Filterable, filter_hint_spans};
use crate::tools::common::{Cmd, draw_list_block, handle_filter_keys, hint_spans_from_cmds, nav_down, nav_up};
use crate::ui::centered_rect;

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

pub const CMDS: &[Cmd] = &[
    Cmd::new("j/k",    "navigate"),
    Cmd::danger("d",   "sign out"),
    Cmd::new("/",      "filter"),
    Cmd::new("r",      "refresh"),
    Cmd::new(":",      "command"),
    Cmd::new("Esc/q",  "home"),
];

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
    pub filter: FilterState,
}

fn filtered_devices(app: &App) -> Vec<&DeviceInfo> {
    app.devices.devices.iter()
        .filter(|d| app.devices.filter.matches_item(*d))
        .collect()
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

    if app.devices.filter.active {
        let filtered_len = filtered_devices(app).len();
        handle_filter_keys(
            &mut app.devices.filter,
            &mut app.devices.selected,
            filtered_len,
            DeviceInfo::filter_cols().len(),
            code,
        );
        return;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
            if !app.devices.filter.input.is_empty() {
                app.devices.filter.clear();
            } else {
                app.active_tool = ActiveTool::Home;
            }
        }
        KeyCode::Char('/') => {
            app.devices.filter.active = true;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let len = filtered_devices(app).len();
            nav_down(&mut app.devices.selected, len);
        }
        KeyCode::Char('k') | KeyCode::Up => nav_up(&mut app.devices.selected),
        KeyCode::Char('d') | KeyCode::Delete => {
            let devs = filtered_devices(app);
            if let Some(dev) = devs.get(app.devices.selected) {
                if dev.is_current {
                    app.devices.error = Some("Cannot delete the current device.".to_owned());
                } else {
                    let id = dev.device_id.clone();
                    drop(devs);
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
    draw_list_block(
        f,
        "Devices",
        &app.devices.devices,
        app.devices.selected,
        &app.devices.filter,
        app.devices.loading,
        true,
        &app.devices.error,
        area,
        "Loading devices…",
        "No devices found. Press 'r' to refresh.",
        |d: &DeviceInfo| {
            let name = d.display_name.as_deref().unwrap_or("(unnamed)").to_owned();
            let current_marker = if d.is_current {
                Span::styled(" ✓", Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD))
            } else {
                Span::raw("")
            };
            let last_info = match (&d.last_seen_ts, &d.last_seen_ip) {
                (Some(ts), Some(ip)) => format!("  {ts}  {ip}"),
                (Some(ts), None)     => format!("  {ts}"),
                (None, Some(ip))     => format!("  {ip}"),
                (None, None)         => String::new(),
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    name,
                    Style::default()
                        .fg(if d.is_current { SUCCESS } else { ratatui::style::Color::White })
                        .add_modifier(if d.is_current { Modifier::BOLD } else { Modifier::empty() }),
                ),
                current_marker,
                Span::styled(format!("  {}", d.device_id), Style::default().fg(MUTED)),
                Span::styled(last_info, Style::default().fg(MUTED)),
            ]))
        },
    );

    if app.devices.delete_dialog.is_some() {
        draw_delete_dialog(f, app);
    }

    if app.devices.filter.active {
        crate::ui::draw_filter_popup(f, &app.devices.filter, area);
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
                    Style::default().fg(ACCENT_DIM).add_modifier(Modifier::BOLD),
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
                    Style::default().fg(DANGER),
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
                Span::styled("  Password: ", Style::default().fg(ACCENT_DIM)),
                Span::styled(
                    "•".repeat(pwd.len()),
                    Style::default().fg(ratatui::style::Color::White),
                ),
                Span::styled("█", Style::default().fg(ACCENT_DIM)),
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

pub fn hint_spans(app: &App) -> Vec<Span<'static>> {
    if app.devices.filter.active {
        return filter_hint_spans(app.devices.filter.column, DeviceInfo::filter_cols());
    }
    if app.devices.delete_dialog.is_some() {
        return vec![];
    }
    hint_spans_from_cmds(CMDS)
}
