pub mod accounts;
pub mod devices;
pub mod help;
pub mod home;
pub mod ignore_list;
pub mod profile;
pub mod rooms;

use ratatui::style::Color;

// ---------------------------------------------------------------------------
// Element design-system palette (dark theme)
// ---------------------------------------------------------------------------

pub const ACCENT: Color = Color::Rgb(13, 189, 139);    // #0DBD8B Element green
pub const ACCENT_DIM: Color = Color::Rgb(77, 216, 168); // #4DD8A8 selected highlight
pub const SUCCESS: Color = Color::Rgb(13, 189, 139);    // same as ACCENT
pub const DANGER: Color = Color::Rgb(240, 105, 111);    // #F0696F
pub const WARNING: Color = Color::Rgb(224, 160, 62);    // #E0A03E
pub const FG: Color = Color::Rgb(237, 239, 242);        // #EDEFF2 primary text
pub const FG2: Color = Color::Rgb(210, 216, 221);       // #D2D8DD secondary text
pub const MUTED: Color = Color::Rgb(115, 125, 133);     // #737D85
pub const MUTED2: Color = Color::Rgb(79, 87, 94);       // #4F575E
pub const BG: Color = Color::Rgb(14, 20, 22);           // #0E1416
pub const BG2: Color = Color::Rgb(20, 24, 27);          // #14181B elevated surface
pub const BG3: Color = Color::Rgb(31, 37, 40);          // #1F2528 tertiary surface
pub const BORDER: Color = Color::Rgb(47, 54, 59);       // #2F363B

// ---------------------------------------------------------------------------
// Shared filter component
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
