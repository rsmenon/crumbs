pub mod dashboard;
pub mod task_list;
pub mod tasks_tab;
pub mod calendar;
pub mod day_view;
pub mod note_view;
pub mod people_view;
pub mod sink_overlay;
pub mod search_overlay;
pub mod command_palette;
pub mod nvim_bridge;
pub mod nvim_overlay;
pub mod date_picker;

use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::Rect;
use crate::app::theme::Theme;
use crate::app::message::AppMessage;

/// Every view in the application implements this trait.
pub trait View {
    /// Handle a terminal event and optionally return a message for the app.
    fn handle_event(&mut self, event: &Event) -> Option<AppMessage>;

    /// Process an app-level message (e.g. Reload, Resize).
    fn handle_message(&mut self, msg: &AppMessage);

    /// Render the view into the given area.
    fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme);

    /// Returns true when the view is in an input-capturing mode
    /// (e.g. inline editing) and global key bindings should be suppressed.
    fn captures_input(&self) -> bool {
        false
    }
}

// ── Shared helpers ──────────────────────────────────────────────────

/// Render a 1-line hint bar into `area` using the sz style:
/// each (key, desc) pair is rendered as `[accent]key[dim]: desc` separated by `  `.
pub fn render_hint_bar<'a>(
    frame: &mut ratatui::Frame,
    area: Rect,
    hints: &[(&'a str, &'a str)],
    theme: &Theme,
) {
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Paragraph;

    let mut spans: Vec<Span<'a>> = Vec::with_capacity(hints.len() * 3);
    for (i, (key, desc)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", theme.dim));
        }
        spans.push(Span::styled(*key, theme.accent));
        spans.push(Span::styled(format!(": {}", desc), theme.dim));
    }
    // Leading space for padding
    let mut all = vec![Span::styled(" ", theme.dim)];
    all.extend(spans);
    frame.render_widget(Paragraph::new(Line::from(all)), area);
}

/// Truncate a string to at most `n` grapheme-approximate characters,
/// appending an ellipsis if truncated.
pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    // find the byte index of the nth char
    match s.char_indices().nth(n.saturating_sub(1)) {
        Some((idx, _)) => format!("{}…", &s[..idx]),
        None => s.to_string(),
    }
}

/// Mask a private title, returning a fixed-width string of asterisks.
/// The width is always 8 so that the mask does not leak title length.
pub fn mask_private(title: &str, max_w: usize) -> String {
    let _ = title; // original title is intentionally ignored
    let stars = "********";
    if max_w >= stars.len() {
        stars.to_string()
    } else {
        stars[..max_w].to_string()
    }
}

/// Detect if text is marked private via a `[p]` prefix or suffix.
///
/// Returns `(cleaned_text, is_private)` where `cleaned_text` has the
/// `[p]` marker stripped (if present).
pub fn detect_private(text: &str) -> (String, bool) {
    let trimmed = text.trim();
    if trimmed.starts_with("[p]") {
        let cleaned = trimmed[3..].trim_start().to_string();
        return (cleaned, true);
    }
    if trimmed.ends_with("[p]") {
        let cleaned = trimmed[..trimmed.len() - 3].trim_end().to_string();
        return (cleaned, true);
    }
    (trimmed.to_string(), false)
}

/// Nerd Font icons used throughout the UI.
pub mod icons {
    pub const TASK: &str = "\u{f0131}";    // 󰄱  nf-md-checkbox_blank_outline
    pub const NOTE: &str = "\u{f0219}";    // 󰈙  nf-md-file_document
    pub const CALENDAR: &str = "\u{f00ed}"; // 󰃭  nf-md-calendar
    pub const MEMORY: &str = "\u{f0004}";  // 󰀄  nf-md-account
    pub const AGENDA: &str = "\u{f03ea}";    // 󰏪  nf-md-pencil
    pub const TAG: &str = "\u{f02c}";       // tag icon
    pub const ARCHIVE: &str = "⊟";           // archived indicator
    pub const PIN: &str = "\u{f0403}";        // 󰐃  nf-md-pin
    pub const BODY: &str = "≡";             // three horizontal bars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate("hi", 10), "hi");
    }

    #[test]
    fn truncate_exact_length() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn truncate_long_string() {
        let t = truncate("hello world", 6);
        assert!(t.ends_with('…'));
        assert!(t.len() <= 8); // 5 ascii + up to 3-byte ellipsis
    }

    #[test]
    fn mask_private_fixed_width() {
        assert_eq!(mask_private("secret title", 20), "********");
    }

    #[test]
    fn mask_private_narrow() {
        assert_eq!(mask_private("secret", 4), "****");
    }

    #[test]
    fn detect_private_prefix() {
        let (text, priv_) = detect_private("[p] my secret");
        assert!(priv_);
        assert_eq!(text, "my secret");
    }

    #[test]
    fn detect_private_suffix() {
        let (text, priv_) = detect_private("my secret [p]");
        assert!(priv_);
        assert_eq!(text, "my secret");
    }

    #[test]
    fn detect_private_none() {
        let (text, priv_) = detect_private("just normal text");
        assert!(!priv_);
        assert_eq!(text, "just normal text");
    }
}
