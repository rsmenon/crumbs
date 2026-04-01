use chrono::NaiveDate;

use crate::domain::{EntityKind, EntityRef};

/// Identifies which field the date-picker was opened for.
#[derive(Debug, Clone)]
pub enum DatePickerContext {
    /// Task due-date field; carries the task ID.
    TaskDue(String),
    /// Agenda date field; carries the agenda ID.
    AgendaDate(String),
}

/// Unified message type for communication between the root `App`,
/// child views, and overlays.
///
/// Views emit `AppMessage` values from `handle_event`; the `App`
/// routes them in `handle_app_message`.  Messages that represent
/// internal view state (scroll position, input buffer, etc.) stay
/// private to the view and never appear here.
#[derive(Debug, Clone)]
pub enum AppMessage {
    // ── Data lifecycle ────────────────────────────────────────────
    /// Ask every view to reload its data from the store.
    Reload,
    /// Terminal was resized.  `height` is the *content* area height
    /// (excluding tab bar + status bar).
    Resize { width: u16, height: u16 },

    // ── Overlay control ───────────────────────────────────────────
    OpenSink,
    CloseSink,
    OpenSearch,
    CloseSearch,
    ClosePalette,
    /// Execute a named palette action (e.g. "new-task", "sync").
    PaletteAction(String),

    // ── Editor integration ────────────────────────────────────────
    /// Open a note in `$EDITOR` by its ID.
    OpenNoteEditor(String),
    /// Open a task in `$EDITOR` by its ID (renders markdown front matter).
    OpenTaskEditor(String),

    // ── Inline editor ─────────────────────────────────────────────
    /// Open the inline tui-textarea editor for a note or task body.
    OpenInlineEditor { kind: EntityKind, id: String },
    /// The inline editor was closed.
    EditorClosed,

    // ── Entity operations ─────────────────────────────────────────
    /// Generic edit request — the app routes by `EntityKind`.
    EditEntity { kind: EntityKind, id: String },
    /// Navigate to a cross-entity reference (jump to task, note, etc.).
    NavigateRef(EntityRef),
    /// Navigate to a person's page in the People tab.
    NavigatePerson(String),

    // ── Error handling ────────────────────────────────────────────
    /// Show an error flash in the status bar (auto-clears after 3 s).
    Error(String),

    // ── Tag filter ──────────────────────────────────────────────────
    /// Global tag filter changed (Some = active filter, None = cleared).
    TagFilterChanged(Option<String>),

    // ── Date picker ───────────────────────────────────────────────
    /// Open the date-picker popup. `date` is the pre-selected date (None = today).
    OpenDatePicker {
        date: Option<NaiveDate>,
        context: DatePickerContext,
    },
    /// The user confirmed a date in the picker.
    DatePickerConfirm(NaiveDate),
    /// The user cancelled the picker without selecting.
    DatePickerCancel,

    // ── System ────────────────────────────────────────────────────
    /// Quit the application.
    Quit,
}
