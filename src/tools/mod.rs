pub mod accounts;
pub mod devices;
pub mod help;
pub mod home;
pub mod ignore_list;
pub mod leave_rooms;
pub mod profile;
pub mod rooms;

use ratatui::style::Color;

// ---------------------------------------------------------------------------
// Shared colour palette (imported by all tool modules and ui.rs)
// ---------------------------------------------------------------------------

pub const ACCENT: Color = Color::Cyan;
pub const FOCUSED: Color = Color::Yellow;
pub const SUCCESS: Color = Color::Green;
pub const ERROR: Color = Color::Red;
pub const MUTED: Color = Color::DarkGray;
pub const BG: Color = Color::Rgb(15, 15, 25);
pub const HEADER_BG: Color = Color::Rgb(25, 25, 45);

// ---------------------------------------------------------------------------
// Shared filter component (used by Leave Rooms and Room Browser)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct FilterState {
    pub active: bool,
    pub input: String,
}

impl FilterState {
    pub fn matches(&self, name: &str) -> bool {
        if !self.active || self.input.is_empty() {
            return true;
        }
        name.to_lowercase().contains(&self.input.to_lowercase())
    }

    pub fn clear(&mut self) {
        self.active = false;
        self.input.clear();
    }
}
