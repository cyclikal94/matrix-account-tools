use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::app::{ActiveTool, App};
use crate::tools::{ACCENT, ACCENT_DIM, BG2, BORDER, MUTED};
use crate::tools::common::{Cmd, legend_help_lines};
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

                lines.extend(legend_help_lines(crate::tools::rooms::LEGEND));
            }
            lines
        }
        ActiveTool::Accounts => {
            use crate::tools::accounts::{AccountTab, CMDS_LIST, CMDS_DETAIL, CMDS_EDITING,
                                          CMDS_DEVICES, CMDS_IGNORED, CMDS_IGNORED_ADD};
            let at = &app.accounts_tool;
            if at.delete_dialog.is_some() || at.ignored_confirm_unignore {
                vec![]
            } else if at.ignored_add_prompt.is_some() {
                from_cmds("Add Ignored User", CMDS_IGNORED_ADD)
            } else if at.detail_open && at.detail_tab_focused {
                match at.active_tab {
                    AccountTab::Devices => from_cmds("Devices Tab", CMDS_DEVICES),
                    AccountTab::IgnoredUsers => from_cmds("Ignored Users Tab", CMDS_IGNORED),
                }
            } else if at.detail_open {
                if at.is_profile_editing() {
                    from_cmds("Editing Profile", CMDS_EDITING)
                } else {
                    from_cmds("Account Detail", CMDS_DETAIL)
                }
            } else {
                from_cmds("Accounts", CMDS_LIST)
            }
        }
    }
}
