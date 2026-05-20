mod app;
mod matrix;
mod tools;
mod ui;

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Rect},
    style::{Color, Style},
    widgets::{Block, Paragraph},
};

use crate::app::App;

#[tokio::main]
async fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run App::new() concurrently, drawing a spinner in the meantime.
    let mut app_task = tokio::spawn(App::new());
    let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let mut frame = 0usize;
    let mut app = loop {
        tokio::select! {
            result = &mut app_task => break result.unwrap(),
            _ = tokio::time::sleep(Duration::from_millis(80)) => {
                let ch = spinner[frame % spinner.len()];
                terminal.draw(|f| draw_loading(f, ch))?;
                frame += 1;
            }
        }
    };

    let result = app.run(&mut terminal).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn draw_loading(f: &mut Frame, spinner: char) {
    const BG: Color = Color::Rgb(14, 20, 22);
    const ACCENT: Color = Color::Rgb(13, 189, 139);

    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(BG)), area);

    let msg = format!("{spinner}  Starting up\u{2026}");
    let w = (msg.chars().count() as u16 + 2).min(area.width);
    let x = area.width.saturating_sub(w) / 2;
    let y = area.height / 2;
    f.render_widget(
        Paragraph::new(msg)
            .style(Style::default().fg(ACCENT).bg(BG))
            .alignment(Alignment::Center),
        Rect::new(x, y, w, 1),
    );
}
