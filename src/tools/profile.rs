use std::time::Instant;

use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use tokio::sync::oneshot;

use crate::app::{ActiveTool, App};
use crate::tools::{ACCENT, ACCENT_DIM, DANGER, MUTED, SUCCESS};
use crate::tools::common::{Cmd, hint_spans_from_cmds};

pub const CMDS: &[Cmd] = &[
    Cmd::new("Tab/j/k", "switch"),
    Cmd::new("e/Enter",  "edit"),
    Cmd::new("r",        "reload"),
    Cmd::new(":",        "command"),
    Cmd::new("Esc/q",    "home"),
];

pub const CMDS_EDITING: &[Cmd] = &[
    Cmd::success("Enter", "save"),
    Cmd::new("Esc",       "discard"),
];

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ProfileField {
    #[default]
    DisplayName,
    AvatarUrl,
}

#[derive(Default)]
pub struct ProfileState {
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub edit_display_name: Option<String>,
    pub edit_avatar_url: Option<String>,
    pub focused: ProfileField,
    pub loading: bool,
    pub saving: bool,
    pub error: Option<String>,
    pub load_rx: Option<oneshot::Receiver<Result<(Option<String>, Option<String>), String>>>,
}

impl ProfileState {
    pub fn is_editing(&self) -> bool {
        self.edit_display_name.is_some() || self.edit_avatar_url.is_some()
    }

    fn active_edit(&mut self) -> Option<&mut String> {
        match self.focused {
            ProfileField::DisplayName => self.edit_display_name.as_mut(),
            ProfileField::AvatarUrl => self.edit_avatar_url.as_mut(),
        }
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn handle(app: &mut App, code: KeyCode) {
    if app.profile.loading || app.profile.saving {
        return;
    }

    if app.profile.is_editing() {
        handle_editing(app, code).await;
        return;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            app.active_tool = ActiveTool::Home;
        }
        KeyCode::Tab | KeyCode::Down | KeyCode::Char('j') => {
            app.profile.focused = match app.profile.focused {
                ProfileField::DisplayName => ProfileField::AvatarUrl,
                ProfileField::AvatarUrl => ProfileField::DisplayName,
            };
        }
        KeyCode::BackTab | KeyCode::Up | KeyCode::Char('k') => {
            app.profile.focused = match app.profile.focused {
                ProfileField::DisplayName => ProfileField::AvatarUrl,
                ProfileField::AvatarUrl => ProfileField::DisplayName,
            };
        }
        KeyCode::Char('e') | KeyCode::Enter => {
            let current = match app.profile.focused {
                ProfileField::DisplayName => app.profile.display_name.clone(),
                ProfileField::AvatarUrl => app.profile.avatar_url.clone(),
            };
            match app.profile.focused {
                ProfileField::DisplayName => {
                    app.profile.edit_display_name = Some(current.unwrap_or_default());
                }
                ProfileField::AvatarUrl => {
                    app.profile.edit_avatar_url = Some(current.unwrap_or_default());
                }
            }
            app.profile.error = None;
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            start_load(app);
        }
        _ => {}
    }
}

async fn handle_editing(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.profile.edit_display_name = None;
            app.profile.edit_avatar_url = None;
        }
        KeyCode::Backspace => {
            if let Some(s) = app.profile.active_edit() {
                s.pop();
            }
        }
        KeyCode::Char(c) if !c.is_control() => {
            if let Some(s) = app.profile.active_edit() {
                s.push(c);
            }
        }
        KeyCode::Enter => {
            do_save(app).await;
        }
        KeyCode::Tab | KeyCode::Down => {
            // commit current field and move to next
            app.profile.focused = match app.profile.focused {
                ProfileField::DisplayName => ProfileField::AvatarUrl,
                ProfileField::AvatarUrl => ProfileField::DisplayName,
            };
        }
        _ => {}
    }
}

async fn do_save(app: &mut App) {
    app.profile.saving = true;
    app.profile.error = None;

    let Some(client) = &app.matrix else {
        app.profile.saving = false;
        app.profile.error = Some("Not connected.".to_owned());
        app.profile.edit_display_name = None;
        app.profile.edit_avatar_url = None;
        return;
    };

    // Save whichever field is being edited.
    let result = match app.profile.focused {
        ProfileField::DisplayName => {
            let val = app.profile.edit_display_name.take().unwrap_or_default();
            let v = if val.is_empty() { None } else { Some(val.as_str()) };
            let r = client.set_display_name(v).await;
            if r.is_ok() {
                app.profile.display_name = if val.is_empty() { None } else { Some(val) };
            }
            r
        }
        ProfileField::AvatarUrl => {
            let val = app.profile.edit_avatar_url.take().unwrap_or_default();
            if !val.is_empty() && !val.starts_with("mxc://") {
                app.profile.error = Some("Avatar URL must start with mxc://".to_owned());
                app.profile.saving = false;
                return;
            }
            let v = if val.is_empty() { None } else { Some(val.as_str()) };
            let r = client.set_avatar_url(v).await;
            if r.is_ok() {
                app.profile.avatar_url = if val.is_empty() { None } else { Some(val) };
            }
            r
        }
    };

    match result {
        Ok(()) => app.toast = Some(("Saved!".to_owned(), SUCCESS, Instant::now())),
        Err(e) => app.profile.error = Some(format!("{e}")),
    }
    app.profile.saving = false;
}

pub fn start_load(app: &mut App) {
    let Some(client) = app.matrix.clone() else { return; };
    app.profile.loading = true;
    app.profile.error = None;
    let (tx, rx) = oneshot::channel();
    app.profile.load_rx = Some(rx);
    tokio::spawn(async move {
        let result = client.get_profile().await.map_err(|e| e.to_string());
        let _ = tx.send(result);
    });
}

pub fn poll_load(app: &mut App) {
    let received = app
        .profile
        .load_rx
        .as_mut()
        .and_then(|rx| rx.try_recv().ok());
    if let Some(result) = received {
        app.profile.load_rx = None;
        match result {
            Ok((dn, av)) => {
                app.profile.display_name = dn;
                app.profile.avatar_url = av;
                app.profile.error = None;
            }
            Err(e) => {
                app.profile.error = Some(e);
            }
        }
        app.profile.loading = false;
    }
}

// ---------------------------------------------------------------------------
// Draw
// ---------------------------------------------------------------------------

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    if app.profile.loading {
        f.render_widget(
            Paragraph::new("Loading profile…")
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let chunks = Layout::vertical([
        Constraint::Length(1), // padding
        Constraint::Length(3), // display name
        Constraint::Length(1), // gap
        Constraint::Length(3), // avatar url
        Constraint::Length(1), // gap
        Constraint::Length(1), // status line
    ])
    .split(area);

    let dn_focused = app.profile.focused == ProfileField::DisplayName;
    let av_focused = app.profile.focused == ProfileField::AvatarUrl;

    let dn_editing = app.profile.edit_display_name.is_some();
    let av_editing = app.profile.edit_avatar_url.is_some();

    let dn_text = if let Some(s) = &app.profile.edit_display_name {
        s.clone()
    } else {
        app.profile
            .display_name
            .clone()
            .unwrap_or_else(|| "(not set)".to_owned())
    };

    let av_text = if let Some(s) = &app.profile.edit_avatar_url {
        s.clone()
    } else {
        app.profile
            .avatar_url
            .clone()
            .unwrap_or_else(|| "(not set)".to_owned())
    };

    let make_field =
        |label: &str, value: &str, focused: bool, editing: bool| -> Paragraph<'static> {
            let border_color = if editing {
                ACCENT_DIM
            } else if focused {
                ACCENT
            } else {
                MUTED
            };
            let text_color = if editing {
                ratatui::style::Color::White
            } else if focused {
                ratatui::style::Color::White
            } else {
                MUTED
            };
            let display = if editing {
                format!("{value}█")
            } else {
                value.to_owned()
            };
            Paragraph::new(display.clone()).block(
                Block::default()
                    .title(Span::styled(
                        format!(" {label} "),
                        Style::default().fg(border_color),
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color)),
            ).style(Style::default().fg(text_color))
        };

    f.render_widget(
        make_field("Display Name", &dn_text, dn_focused, dn_editing),
        chunks[1],
    );
    f.render_widget(
        make_field("Avatar URL", &av_text, av_focused, av_editing),
        chunks[3],
    );

    // Status line.
    let status: Paragraph = if app.profile.saving {
        Paragraph::new("Saving…")
            .style(Style::default().fg(ACCENT).add_modifier(Modifier::ITALIC))
            .alignment(Alignment::Center)
    } else if let Some(err) = &app.profile.error {
        Paragraph::new(err.as_str())
            .style(Style::default().fg(DANGER))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
    } else {
        Paragraph::new("")
    };
    f.render_widget(status, chunks[5]);
}

pub fn hint_spans(app: &App) -> Vec<Span<'static>> {
    if app.profile.is_editing() {
        hint_spans_from_cmds(CMDS_EDITING)
    } else {
        hint_spans_from_cmds(CMDS)
    }
}

