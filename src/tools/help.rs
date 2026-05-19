use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::tools::{ACCENT, FOCUSED, MUTED};
use crate::ui::centered_rect;

pub fn draw_overlay(f: &mut Frame) {
    let area = f.area();
    let popup = centered_rect(62, 36, area);
    f.render_widget(Clear, popup);

    let section = |title: &'static str| -> Line<'static> {
        Line::from(vec![Span::styled(
            format!("  {title}"),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )])
    };

    let row = |key: &'static str, desc: &'static str| -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("    {key:<18}"), Style::default().fg(FOCUSED)),
            Span::styled(desc, Style::default().fg(ratatui::style::Color::White)),
        ])
    };

    let lines: Vec<Line> = vec![
        Line::from(""),
        section("Global"),
        row("Ctrl+C", "Quit"),
        row(":", "Open command bar"),
        row("?", "Toggle this help"),
        Line::from(""),
        section("Command bar"),
        row(":home / :h", "Go to home screen"),
        row(":leaverooms / :lr", "Leave rooms tool"),
        row(":rooms", "Room browser"),
        row(":accounts", "Account manager"),
        row(":ignorelist", "Ignore list"),
        row(":profile", "Profile editor"),
        row(":devices", "Device manager"),
        row(":help", "Show this help"),
        row(":login", "Add a new account"),
        row(":quit / :q", "Quit"),
        Line::from(""),
        section("Lists (Leave Rooms, Rooms, Accounts)"),
        row("j / k  ↑↓", "Navigate"),
        row("Space", "Toggle checkbox (Leave Rooms)"),
        row("Enter", "Confirm / open detail"),
        row("/", "Filter (Leave Rooms, Rooms)"),
        row("r", "Refresh"),
        row("Esc / q", "Back to home"),
        Line::from(""),
        section("Profile"),
        row("Tab / j / k", "Switch field"),
        row("e / Enter", "Start editing field"),
        row("Enter (editing)", "Save field"),
        row("Esc (editing)", "Discard edits"),
        Line::from(""),
        section("Accounts / Devices"),
        row("a", "Add account"),
        row("d / Delete", "Remove account / sign out device"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Press Esc or ? to close",
            Style::default().fg(MUTED),
        )]),
    ];

    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(Span::styled(
                    " Keyboard Shortcuts ",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ACCENT))
                .style(Style::default().bg(ratatui::style::Color::Rgb(18, 18, 32))),
        ),
        popup,
    );
}
