use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::app::{ActiveTool, App};
use crate::tools::{ACCENT, ACCENT_DIM, BG2, BORDER, MUTED};
use crate::tools::common::Cmd;
use crate::ui::centered_rect;

pub fn draw_overlay(f: &mut Frame, app: &App) {
    let tool_lines = tool_help_lines(app);
    let total = 3 /* top blank + global header + blank */
        + 3 /* global rows */
        + 1 /* blank */
        + tool_lines.len()
        + 2; /* blank + close hint */

    let height = (total as u16 + 2).min(f.area().height.saturating_sub(2));
    let popup = centered_rect(58, height, f.area());
    f.render_widget(Clear, popup);

    let section = |title: &'static str| -> Line<'static> {
        Line::from(Span::styled(
            format!("  {title}"),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
    };

    let row = |key: &'static str, desc: &'static str| -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("    {key:<18}"), Style::default().fg(ACCENT_DIM)),
            Span::styled(desc, Style::default().fg(ratatui::style::Color::Rgb(237, 239, 242))),
        ])
    };

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        section("Global"),
        row("Ctrl+C", "Quit"),
        row(":", "Open command bar"),
        row("?", "Toggle help"),
        Line::from(""),
    ];

    lines.extend(tool_lines);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Press Esc or ? to close",
        Style::default().fg(MUTED),
    )));

    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(Span::styled(
                    " Keyboard Shortcuts ",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER))
                .style(Style::default().bg(BG2)),
        ),
        popup,
    );
}

fn tool_help_lines(app: &App) -> Vec<Line<'static>> {
    let section = |title: &'static str| -> Line<'static> {
        Line::from(Span::styled(
            format!("  {title}"),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
    };
    let row = |key: &'static str, desc: &'static str| -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("    {key:<18}"), Style::default().fg(ACCENT_DIM)),
            Span::styled(desc, Style::default().fg(ratatui::style::Color::Rgb(237, 239, 242))),
        ])
    };
    let from_cmds = |title: &'static str, cmds: &[Cmd]| -> Vec<Line<'static>> {
        let mut lines = vec![section(title)];
        for cmd in cmds {
            lines.push(Line::from(vec![
                Span::styled(format!("    {:<18}", cmd.key), Style::default().fg(ACCENT_DIM)),
                Span::styled(cmd.desc, Style::default().fg(ratatui::style::Color::Rgb(237, 239, 242))),
            ]));
        }
        lines
    };

    match app.active_tool {
        ActiveTool::Home => vec![
            section("Home"),
            row("↑↓ / k j", "Navigate rows"),
            row("←→ / h l", "Navigate columns"),
            row("Enter", "Open selected tool"),
            row("q", "Quit"),
        ],
        ActiveTool::Rooms => {
            let mut lines: Vec<Line> = Vec::new();

            // Context-specific shortcuts based on active sub-view.
            if app.rooms_tool.member_profile.is_some() {
                lines.push(section("Member Profile"));
                lines.push(row("i", "Copy MXID to clipboard"));
                lines.push(row("I", "Ignore this user"));
                lines.push(row("Esc", "Back to member list"));
            } else if app.rooms_tool.detail_members_focused {
                lines.push(section("Members"));
                lines.push(row("j / k  ↑↓", "Navigate"));
                lines.push(row("Enter / e", "View member profile"));
                lines.push(row("i", "Copy MXID to clipboard"));
                lines.push(row("I", "Ignore selected member"));
                lines.push(row("p", "Set power level"));
                lines.push(row("K", "Kick member"));
                lines.push(row("b", "Ban member"));
                lines.push(row("r", "Refresh member list"));
                lines.push(row("d", "Back to room detail"));
                lines.push(row("Esc / q", "Back to room list"));
            } else if app.rooms_tool.detail_open {
                lines.push(section("Room Detail"));
                lines.push(row("j / k  ↑↓", "Navigate fields"));
                lines.push(row("e / Enter", "Edit focused field"));
                lines.push(row("PgDn / PgUp", "Scroll topic"));
                lines.push(row("i", "Copy room ID to clipboard"));
                lines.push(row("m", "Focus members list"));
                lines.push(row("x", "Leave this room"));
                lines.push(row("Esc / q", "Back to room list"));
            } else if app.rooms_tool.leave_select {
                lines.push(section("Leave Select"));
                lines.push(row("j / k  ↑↓", "Navigate rooms"));
                lines.push(row("Space", "Toggle selection"));
                lines.push(row("Enter", "Leave selected rooms"));
                lines.push(row("Esc", "Cancel"));
            } else {
                lines.push(section("Room List"));
                lines.push(row("j / k  ↑↓", "Navigate"));
                lines.push(row("/", "Filter rooms"));
                lines.push(row("Enter / e / d", "Open room detail"));
                lines.push(row("m", "Open members list"));
                lines.push(row("x", "Enter leave-select mode"));
                lines.push(row("r", "Refresh"));
                lines.push(row("Esc / q", "Back to home"));

                // Legend — only relevant when the room list is visible.
                lines.push(Line::from(""));
                lines.push(section("Legend"));
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled("●", Style::default().fg(ACCENT)),
                    Span::styled(
                        format!("{:<17}", " (green)"),
                        Style::default().fg(ACCENT_DIM),
                    ),
                    Span::styled(
                        "end-to-end encrypted",
                        Style::default().fg(ratatui::style::Color::Rgb(237, 239, 242)),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled("●", Style::default().fg(MUTED)),
                    Span::styled(
                        format!("{:<17}", " (grey)"),
                        Style::default().fg(ACCENT_DIM),
                    ),
                    Span::styled(
                        "not end-to-end encrypted",
                        Style::default().fg(ratatui::style::Color::Rgb(237, 239, 242)),
                    ),
                ]));
                lines.push(row("dm", "direct message"));
            }
            lines
        }
        ActiveTool::Accounts  => from_cmds("Accounts",    crate::tools::accounts::CMDS),
        ActiveTool::IgnoreList => from_cmds("Ignore List", crate::tools::ignore_list::CMDS),
        ActiveTool::Devices    => from_cmds("Devices",     crate::tools::devices::CMDS),
        ActiveTool::Profile => {
            let mut lines = from_cmds("Profile", crate::tools::profile::CMDS);
            lines.push(Line::from(""));
            lines.extend(from_cmds("While Editing", crate::tools::profile::CMDS_EDITING));
            lines
        }
    }
}
