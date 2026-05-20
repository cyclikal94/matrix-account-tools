pub mod accounts;
pub mod devices;
pub mod help;
pub mod home;
pub mod ignore_list;
pub mod profile;
pub mod rooms;

use ratatui::style::{Color, Style};
use ratatui::text::Span;

// ---------------------------------------------------------------------------
// Element design-system palette (dark theme)
// ---------------------------------------------------------------------------

pub const ACCENT: Color = Color::Rgb(13, 189, 139);    // #0DBD8B Element green
pub const ACCENT_DIM: Color = Color::Rgb(77, 216, 168); // #4DD8A8 selected highlight
pub const SUCCESS: Color = Color::Rgb(13, 189, 139);    // same as ACCENT
pub const DANGER: Color = Color::Rgb(240, 105, 111);    // #F0696F
#[allow(dead_code)]
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

/// Implement this on any list-item type to get column-aware filtering for free.
///
/// `filter_cols()[0]` must always be "all" (the implicit "search everything" option).
/// `filter_value(col)` returns the string to match for that column (col ≥ 1).
pub trait Filterable {
    fn filter_cols() -> &'static [&'static str];
    fn filter_value(&self, col: usize) -> String;
}

#[derive(Debug, Default, Clone)]
pub struct FilterState {
    pub active: bool,
    pub input: String,
    /// None / Some(0) = all columns, Some(n) = column n (1-based).
    pub column: Option<usize>,
}

impl FilterState {
    pub fn matches(&self, name: &str) -> bool {
        if self.input.is_empty() {
            return true;
        }
        name.to_lowercase().contains(&self.input.to_lowercase())
    }

    /// Returns true if `item` matches the current filter input and column selection.
    pub fn matches_item<T: Filterable>(&self, item: &T) -> bool {
        if self.input.is_empty() {
            return true;
        }
        let col = self.column.unwrap_or(0);
        if col == 0 {
            (1..T::filter_cols().len()).any(|i| self.matches(&item.filter_value(i)))
        } else {
            self.matches(&item.filter_value(col))
        }
    }

    pub fn clear(&mut self) {
        self.active = false;
        self.input.clear();
        self.column = None;
    }
}

/// Standard bottom-bar hint spans to show while a filter is active.
/// `active_col` is `filter.column` (None = all / 0). `cols` is the item type's `filter_cols()`.
pub fn filter_hint_spans(active_col: Option<usize>, cols: &[&str]) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = vec![
        Span::styled("type", Style::default().fg(ACCENT)),
        Span::raw(" to filter"),
    ];
    if cols.len() > 1 {
        let active = active_col.unwrap_or(0);
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("[0-{}]", cols.len() - 1),
            Style::default().fg(ACCENT),
        ));
        spans.push(Span::raw(" ("));
        for (i, label) in cols.iter().enumerate() {
            if i > 0 { spans.push(Span::raw("  ")); }
            spans.push(Span::styled(
                format!("{i}:{label}"),
                Style::default().fg(if active == i { ACCENT } else { MUTED }),
            ));
        }
        spans.push(Span::raw(")"));
    }
    spans.extend([
        Span::raw("  "),
        Span::styled("Enter", Style::default().fg(ACCENT)),
        Span::raw(" close  "),
        Span::styled("Esc", Style::default().fg(ACCENT)),
        Span::raw(" clear"),
    ]);
    spans
}
