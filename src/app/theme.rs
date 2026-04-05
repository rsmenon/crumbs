use chrono::NaiveDate;
use ratatui::style::{Color, Modifier, Style};

use crate::domain::TaskStatus;

// ── Gruvbox Dark true-color palette ───────────────────────────────
//
// Uses exact Gruvbox RGB values for true-color terminals.
// Falls back gracefully in 256-color terminals (slight color shift).
//
// Reference: https://github.com/morhetz/gruvbox

// Accent colors (Gruvbox dark variants)
const BLUE: Color = Color::Rgb(0x45, 0x85, 0x88);     // #458588
const AQUA: Color = Color::Rgb(0x68, 0x9d, 0x6a);     // #689d6a
const YELLOW: Color = Color::Rgb(0xd7, 0x99, 0x21);    // #d79921
const RED: Color = Color::Rgb(0xcc, 0x24, 0x1d);       // #cc241d
const GREEN: Color = Color::Rgb(0x98, 0x97, 0x1a);     // #98971a
const ORANGE: Color = Color::Rgb(0xd6, 0x5d, 0x0e);    // #d65d0e
const PURPLE: Color = Color::Rgb(0xb1, 0x62, 0x86);    // #b16286

// Backgrounds
/// Background 0 — darkest (main bg). Not set explicitly so terminal default shows through.
const BG0: Color = Color::Rgb(0x28, 0x28, 0x28);       // #282828
/// Background 1 — slightly lighter. Selected rows.
const BG1: Color = Color::Rgb(0x3c, 0x38, 0x36);       // #3c3836
/// Background 2 — borders and separators.
const BG2: Color = Color::Rgb(0x50, 0x49, 0x45);       // #504945
/// Background for column-focus highlight.
const BG_COL_FOCUS: Color = Color::Rgb(0x66, 0x5c, 0x54); // #665c54 (bg3)
/// Background for the "selected item" highlight.
const BG_SELECTED: Color = Color::Rgb(0x3c, 0x38, 0x36);  // #3c3836 (bg1)

// Foregrounds
/// Foreground 0 — primary text.
const FG0: Color = Color::Rgb(0xeb, 0xdb, 0xb2);       // #ebdbb2
/// Foreground 4 — dimmed / secondary text (neutral gray, no brown tint).
const FG4: Color = Color::Rgb(0xa8, 0x99, 0x84);       // #a89984
/// Very dim text for archived / inactive elements.
const FG_DIM: Color = Color::Rgb(0x7c, 0x6f, 0x64);    // #7c6f64 (gray/bg4)

/// A full set of pre-computed `Style` values for the Gruvbox Dark
/// color scheme.  Constructed once at startup via
/// [`Theme::gruvbox_dark()`] and passed by reference to every
/// `draw()` call.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Theme {
    // ── Tab bar ───────────────────────────────────────────────────
    /// Active tab label style.
    pub tab_active: Style,
    /// Inactive tab label style.
    pub tab_inactive: Style,

    // ── Typography ────────────────────────────────────────────────
    /// Top-level section titles.
    pub title: Style,
    /// Smaller headings / subtitles.
    pub subtitle: Style,
    /// Accent for highlighted elements (uses blue).
    pub accent: Style,

    // ── Semantic ──────────────────────────────────────────────────
    /// Error messages and destructive actions.
    pub error: Style,
    /// Success / positive feedback.
    pub success: Style,
    /// Warnings / caution.
    pub warning: Style,
    /// Dimmed / de-emphasized text.
    pub dim: Style,

    // ── Entity annotations ────────────────────────────────────────
    /// @person mentions.
    pub person: Style,
    /// #topic tags.
    pub topic: Style,

    // ── Structural ────────────────────────────────────────────────
    /// Borders and separator lines.
    pub border: Style,
    /// Selected / highlighted row background.
    pub selected: Style,
    /// Selected row inside an overlay (popup bg is BG1, so darker BG0 creates contrast).
    pub selected_overlay: Style,
    /// Column header labels.
    pub column_header: Style,
    /// Background highlight for the focused column cell.
    pub column_focus: Style,
    /// Row background tint for the cursor row (distinct from column
    /// focus so both can be visible simultaneously).
    pub row_gray: Style,

    // ── Status bar ────────────────────────────────────────────────
    /// Key-hint text in the bottom status bar.
    pub status_bar: Style,

    // ── Overlays ──────────────────────────────────────────────────
    /// Background for floating panels (one shade lighter than terminal bg).
    pub popup_bg: Style,

    // ── Misc ──────────────────────────────────────────────────────
    /// Private / locked entries.
    pub private: Style,
    /// Dates and timestamps.
    pub date: Style,
    /// Priority labels.
    pub priority_high: Style,
    pub priority_medium: Style,
    pub priority_low: Style,

    // ── Task statuses ─────────────────────────────────────────────
    pub status_backlog: Style,
    pub status_todo: Style,
    pub status_in_progress: Style,
    pub status_blocked: Style,
    pub status_done: Style,
    pub status_archived: Style,

    // ── Cursor ────────────────────────────────────────────────────
    /// Cursor / caret color (Gruvbox light0 fg0).
    pub cursor: Color,
}

impl Theme {
    /// Build the canonical Gruvbox Dark theme.
    pub fn gruvbox_dark() -> Self {
        Self {
            // Tab bar
            tab_active: Style::default()
                .fg(FG4)
                .add_modifier(Modifier::BOLD),
            tab_inactive: Style::default().fg(FG_DIM),

            // Typography
            title: Style::default()
                .fg(FG0)
                .add_modifier(Modifier::BOLD),
            subtitle: Style::default()
                .fg(FG4)
                .add_modifier(Modifier::BOLD),
            accent: Style::default().fg(AQUA),

            // Semantic
            error: Style::default()
                .fg(RED)
                .add_modifier(Modifier::BOLD),
            success: Style::default().fg(GREEN),
            warning: Style::default().fg(ORANGE),
            dim: Style::default().fg(FG_DIM),

            // Entity annotations
            person: Style::default().fg(AQUA),
            topic: Style::default().fg(PURPLE),

            // Structural
            border: Style::default().fg(BG2),
            selected: Style::default().bg(BG_SELECTED).fg(FG0),
            selected_overlay: Style::default().bg(BG0).fg(FG0),
            column_header: Style::default()
                .fg(FG4),
            column_focus: Style::default().bg(BG_COL_FOCUS).fg(FG0),
            row_gray: Style::default().bg(BG1).fg(FG0),

            // Status bar
            status_bar: Style::default().fg(FG4),

            // Overlays
            popup_bg: Style::default().bg(BG1),

            // Misc
            private: Style::default().fg(PURPLE),
            date: Style::default().fg(FG0),
            priority_high: Style::default()
                .fg(RED)
                .add_modifier(Modifier::BOLD),
            priority_medium: Style::default().fg(ORANGE),
            priority_low: Style::default().fg(FG4),

            // Task statuses
            status_backlog: Style::default().fg(FG4),
            status_todo: Style::default().fg(BLUE),
            status_in_progress: Style::default().fg(YELLOW),
            status_blocked: Style::default()
                .fg(RED)
                .add_modifier(Modifier::BOLD),
            status_done: Style::default()
                .fg(GREEN)
                .add_modifier(Modifier::CROSSED_OUT),
            status_archived: Style::default().fg(FG_DIM),

            cursor: FG0,
        }
    }

    /// Return foreground-only style for inline status labels (no
    /// strikethrough on the label itself, that is only for titles).
    pub fn status_fg(&self, status: &TaskStatus) -> Style {
        match status {
            TaskStatus::Backlog => self.status_backlog,
            TaskStatus::Todo => self.status_todo,
            TaskStatus::InProgress => Style::default().fg(YELLOW),
            TaskStatus::Blocked => Style::default().fg(RED),
            TaskStatus::Done => Style::default().fg(GREEN),
            TaskStatus::Archived => self.status_archived,
        }
    }

    /// Return the appropriate style for a due date:
    /// - past due or today → red
    /// - within 3 days → yellow
    /// - otherwise → foreground (same as `date`)
    pub fn due_date_style(&self, due_date: Option<&str>, today: NaiveDate) -> Style {
        match due_date.and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) {
            None => self.date,
            Some(due) => {
                let days = (due - today).num_days();
                if days <= 0 {
                    Style::default().fg(RED).add_modifier(Modifier::BOLD)
                } else if days <= 3 {
                    Style::default().fg(YELLOW)
                } else {
                    self.date
                }
            }
        }
    }

}
