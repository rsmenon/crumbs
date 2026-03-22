use std::collections::HashSet;
use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::message::AppMessage;
use crate::app::theme::Theme;
use crate::domain::{Note, Refs};
use super::{icons, mask_private, truncate, View};
use crate::store::Store;

// ── Column enum ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Title,
    Tags,
    Modified,
}

impl Column {
    fn next(self) -> Self {
        match self {
            Column::Title => Column::Tags,
            Column::Tags => Column::Modified,
            Column::Modified => Column::Title,
        }
    }

    fn prev(self) -> Self {
        match self {
            Column::Title => Column::Modified,
            Column::Tags => Column::Title,
            Column::Modified => Column::Tags,
        }
    }
}

// ── Sort direction ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortDirection {
    Ascending,
    Descending,
}

// ── NoteView ─────────────────────────────────────────────────────

pub struct NoteView {
    store: Arc<dyn Store>,
    notes: Vec<Note>,
    cursor: usize,
    column: Column,
    /// Inline editing state.
    editing: bool,
    edit_buf: String,
    edit_cursor: usize,
    /// Column-based sort (None = unsorted / original load order).
    sort_column: Option<Column>,
    sort_direction: Option<SortDirection>,
    /// Preview pane toggle.
    show_preview: bool,
    /// Set of revealed private entry ids.
    revealed: HashSet<String>,
    content_width: u16,
    content_height: u16,
    /// Show archived notes.
    show_archived: bool,
    /// Global tag filter.
    tag_filter: Option<String>,
    /// Index of note awaiting delete confirmation.
    confirm_delete: Option<usize>,
    /// Pending new note being titled inline (not yet saved).
    creating: Option<Note>,
}

impl NoteView {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self {
            store,
            notes: Vec::new(),
            cursor: 0,
            column: Column::Title,
            editing: false,
            edit_buf: String::new(),
            edit_cursor: 0,
            sort_column: None,
            sort_direction: None,
            show_preview: false,
            revealed: HashSet::new(),
            content_width: 80,
            content_height: 24,
            show_archived: false,
            tag_filter: None,
            confirm_delete: None,
            creating: None,
        }
    }

    fn reload(&mut self) {
        let mut notes = self
            .store
            .list_notes()
            .unwrap_or_default();
        if !self.show_archived {
            notes.retain(|n| !n.archived);
        }
        if let Some(ref tag) = self.tag_filter {
            notes.retain(|n| n.refs.topics.iter().any(|t| t == tag));
        }
        self.notes = notes;
        self.sort_notes();
        if !self.notes.is_empty() && self.cursor >= self.notes.len() {
            self.cursor = self.notes.len() - 1;
        }
    }

    fn sort_notes(&mut self) {
        let (col, dir) = match (self.sort_column, self.sort_direction) {
            (Some(c), Some(d)) => (c, d),
            _ => return, // unsorted — keep original load order
        };

        let flip = |o: std::cmp::Ordering| -> std::cmp::Ordering {
            match dir {
                SortDirection::Ascending => o,
                SortDirection::Descending => o.reverse(),
            }
        };

        match col {
            Column::Title => {
                self.notes.sort_by(|a, b| {
                    flip(a.title.to_lowercase().cmp(&b.title.to_lowercase()))
                });
            }
            Column::Tags => {
                self.notes.sort_by(|a, b| {
                    let tag_a = a.refs.topics.first().map(|t| t.to_lowercase());
                    let tag_b = b.refs.topics.first().map(|t| t.to_lowercase());
                    // no-tags always last regardless of direction
                    if tag_a.is_none() && tag_b.is_some() {
                        return std::cmp::Ordering::Greater;
                    }
                    if tag_a.is_some() && tag_b.is_none() {
                        return std::cmp::Ordering::Less;
                    }
                    let ord = match (&tag_a, &tag_b) {
                        (Some(ta), Some(tb)) => ta.cmp(tb),
                        _ => std::cmp::Ordering::Equal,
                    };
                    flip(ord)
                });
            }
            Column::Modified => {
                // Ascending = newest first (more intuitive for dates),
                // Descending = oldest first.
                self.notes.sort_by(|a, b| {
                    flip(b.updated_at.cmp(&a.updated_at))
                });
            }
        }
    }

    fn current_note(&self) -> Option<&Note> {
        self.notes.get(self.cursor)
    }

    // ── Editing ──────────────────────────────────────────────────

    fn start_edit(&mut self) {
        let buf = {
            let Some(note) = self.notes.get(self.cursor) else {
                return;
            };
            match self.column {
                Column::Title => note.title.clone(),
                Column::Tags => {
                    let mut parts = Vec::new();
                    for t in &note.refs.topics {
                        parts.push(format!("#{}", t));
                    }
                    parts.join(" ")
                }
                Column::Modified => {
                    // Modified is read-only; show current value but don't allow editing
                    note.updated_at.format("%Y-%m-%d %H:%M").to_string()
                }
            }
        };
        self.editing = true;
        self.edit_buf = buf;
        self.edit_cursor = self.edit_buf.len();
    }

    fn save_edit(&mut self) {
        self.editing = false;
        let Some(note) = self.notes.get_mut(self.cursor) else {
            return;
        };

        match self.column {
            Column::Title => {
                note.title = self.edit_buf.clone();
            }
            Column::Tags => {
                let mut people = Vec::new();
                let mut topics = Vec::new();
                for token in self.edit_buf.split_whitespace() {
                    if let Some(p) = token.strip_prefix('@') {
                        if !p.is_empty() {
                            people.push(p.to_string());
                        }
                    } else if let Some(t) = token.strip_prefix('#') {
                        if !t.is_empty() {
                            topics.push(t.to_string());
                        }
                    }
                }
                note.refs.people = people;
                note.refs.topics = topics;
            }
            Column::Modified => {
                // Modified is read-only; ignore edits
                self.edit_buf.clear();
                return;
            }
        }

        note.updated_at = chrono::Utc::now();
        let _ = self.store.save_note(note);
        self.edit_buf.clear();
    }

    fn cancel_edit(&mut self) {
        self.editing = false;
        self.edit_buf.clear();
        self.edit_cursor = 0;
    }

    fn start_creating(&mut self) {
        let now = chrono::Utc::now();
        let note = Note {
            id: crate::domain::new_id(),
            title: String::new(),
            created_at: now,
            updated_at: now,
            private: false,
            pinned: false,
            archived: false,
            created_dir: std::env::current_dir()
                .ok()
                .and_then(|p| p.to_str().map(|s| s.to_string()))
                .unwrap_or_default(),
            refs: Refs::default(),
            body: String::new(),
        };
        self.creating = Some(note);
        self.cursor = 0;
        self.column = Column::Title;
        self.editing = true;
        self.edit_buf = String::new();
        self.edit_cursor = 0;
    }

    // ── Annotation rendering ─────────────────────────────────────

    fn annotated_tags<'a>(note: &Note, theme: &'a Theme) -> Vec<Span<'a>> {
        let mut spans = Vec::new();
        for t in &note.refs.topics {
            if !spans.is_empty() {
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(format!("#{}", t), theme.topic));
        }
        spans
    }
}

// ── View trait ────────────────────────────────────────────────────

impl View for NoteView {
    fn handle_event(&mut self, event: &Event) -> Option<AppMessage> {
        let Event::Key(KeyEvent { code, .. }) = event else {
            return None;
        };

        if self.confirm_delete.is_some() {
            return self.handle_confirm_delete_key(*code);
        }

        if self.editing {
            return self.handle_edit_key(*code);
        }

        self.handle_normal_key(*code)
    }

    fn handle_message(&mut self, msg: &AppMessage) {
        match msg {
            AppMessage::Reload => {
                self.reload();
            }
            AppMessage::TagFilterChanged(filter) => {
                self.tag_filter = filter.clone();
            }
            AppMessage::Resize { width, height } => {
                self.content_width = *width;
                self.content_height = *height;
            }
            _ => {}
        }
    }

    fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if self.notes.is_empty() && self.creating.is_none() {
            let empty = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled("No notes yet", theme.title)),
                Line::from(""),
                Line::from(Span::styled("Press 'n' to create a new note", theme.dim)),
            ])
            .alignment(ratatui::layout::Alignment::Center);
            frame.render_widget(empty, area);
            return;
        }

        if self.show_preview {
            let confirm_h = if self.confirm_delete.is_some() { 1 } else { 0 };
            // Split 50/50 vertically, with optional confirm bar
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(if confirm_h > 0 {
                    vec![Constraint::Percentage(50), Constraint::Percentage(50), Constraint::Length(1)]
                } else {
                    vec![Constraint::Percentage(50), Constraint::Percentage(50)]
                })
                .split(area);
            self.draw_list(frame, chunks[0], theme);
            self.draw_preview(frame, chunks[1], theme);
            if confirm_h > 0 && chunks.len() > 2 {
                self.draw_confirm_bar(frame, chunks[2], theme);
            }
        } else if self.confirm_delete.is_some() {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(area);
            self.draw_list(frame, chunks[0], theme);
            self.draw_confirm_bar(frame, chunks[1], theme);
        } else {
            self.draw_list(frame, area, theme);
        }
    }

    fn captures_input(&self) -> bool {
        self.editing || self.confirm_delete.is_some()
    }
}

impl NoteView {
    fn handle_normal_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        if self.notes.is_empty() {
            return match code {
                KeyCode::Char('n') => {
                    self.start_creating();
                    None
                }
                KeyCode::Char('A') => {
                    self.show_archived = !self.show_archived;
                    self.reload();
                    None
                }
                _ => None,
            };
        }

        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if self.cursor + 1 < self.notes.len() {
                    self.cursor += 1;
                }
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
                None
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.column = self.column.prev();
                None
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.column = self.column.next();
                None
            }
            KeyCode::Char('g') => {
                self.cursor = 0;
                None
            }
            KeyCode::Char('G') => {
                if !self.notes.is_empty() {
                    self.cursor = self.notes.len() - 1;
                }
                None
            }
            KeyCode::Char('e') => {
                self.start_edit();
                None
            }
            KeyCode::Enter => {
                if let Some(note) = self.current_note() {
                    let note_id = note.id.clone();
                    let note_private = note.private;
                    if note_private {
                        if self.revealed.contains(&note_id) {
                            self.revealed.remove(&note_id);
                        } else {
                            self.revealed.insert(note_id);
                        }
                        return None;
                    }
                    return Some(AppMessage::OpenNoteEditor(note_id));
                }
                None
            }
            KeyCode::Char('n') => {
                self.start_creating();
                None
            }
            KeyCode::Char('p') => {
                if let Some(note) = self.notes.get(self.cursor).cloned() {
                    let mut updated = note;
                    updated.pinned = !updated.pinned;
                    updated.updated_at = chrono::Utc::now();
                    let _ = self.store.save_note(&updated);
                    self.reload();
                }
                Some(AppMessage::Reload)
            }
            KeyCode::Char('v') => {
                self.show_preview = !self.show_preview;
                None
            }
            KeyCode::Char('S') => {
                if self.sort_column == Some(self.column) {
                    // Same column — cycle direction
                    match self.sort_direction {
                        Some(SortDirection::Ascending) => {
                            self.sort_direction = Some(SortDirection::Descending);
                        }
                        Some(SortDirection::Descending) => {
                            self.sort_column = None;
                            self.sort_direction = None;
                        }
                        None => {
                            self.sort_direction = Some(SortDirection::Ascending);
                        }
                    }
                } else {
                    // Different column — start ascending
                    self.sort_column = Some(self.column);
                    self.sort_direction = Some(SortDirection::Ascending);
                }
                self.reload();
                None
            }
            KeyCode::Char('d') => {
                if self.cursor < self.notes.len() {
                    self.confirm_delete = Some(self.cursor);
                }
                None
            }
            KeyCode::Char('a') => {
                if let Some(note) = self.notes.get(self.cursor).cloned() {
                    let mut updated = note;
                    updated.archived = !updated.archived;
                    if updated.archived {
                        updated.pinned = false;
                    }
                    updated.updated_at = chrono::Utc::now();
                    let _ = self.store.save_note(&updated);
                    self.reload();
                }
                Some(AppMessage::Reload)
            }
            KeyCode::Char('A') => {
                self.show_archived = !self.show_archived;
                self.reload();
                None
            }
            _ => None,
        }
    }

    fn handle_edit_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        match code {
            KeyCode::Enter => {
                if let Some(mut note) = self.creating.take() {
                    let title = self.edit_buf.trim().to_string();
                    note.title = if title.is_empty() { "Untitled".to_string() } else { title };
                    note.updated_at = chrono::Utc::now();
                    let id = note.id.clone();
                    let _ = self.store.save_note(&note);
                    self.editing = false;
                    self.edit_buf.clear();
                    self.edit_cursor = 0;
                    self.reload();
                    if let Some(pos) = self.notes.iter().position(|n| n.id == id) {
                        self.cursor = pos;
                    }
                    return Some(AppMessage::OpenNoteEditor(id));
                }
                self.save_edit();
                Some(AppMessage::Reload)
            }
            KeyCode::Esc => {
                self.creating = None;
                self.cancel_edit();
                None
            }
            KeyCode::Left => {
                if self.edit_cursor > 0 {
                    self.edit_cursor -= 1;
                    while self.edit_cursor > 0 && !self.edit_buf.is_char_boundary(self.edit_cursor) {
                        self.edit_cursor -= 1;
                    }
                }
                None
            }
            KeyCode::Right => {
                if self.edit_cursor < self.edit_buf.len() {
                    self.edit_cursor += 1;
                    while self.edit_cursor < self.edit_buf.len() && !self.edit_buf.is_char_boundary(self.edit_cursor) {
                        self.edit_cursor += 1;
                    }
                }
                None
            }
            KeyCode::Home => {
                self.edit_cursor = 0;
                None
            }
            KeyCode::End => {
                self.edit_cursor = self.edit_buf.len();
                None
            }
            KeyCode::Backspace => {
                if self.edit_cursor > 0 {
                    let mut prev = self.edit_cursor - 1;
                    while prev > 0 && !self.edit_buf.is_char_boundary(prev) {
                        prev -= 1;
                    }
                    self.edit_buf.drain(prev..self.edit_cursor);
                    self.edit_cursor = prev;
                }
                None
            }
            KeyCode::Delete => {
                if self.edit_cursor < self.edit_buf.len() {
                    let mut next = self.edit_cursor + 1;
                    while next < self.edit_buf.len() && !self.edit_buf.is_char_boundary(next) {
                        next += 1;
                    }
                    self.edit_buf.drain(self.edit_cursor..next);
                }
                None
            }
            KeyCode::Char(c) => {
                self.edit_buf.insert(self.edit_cursor, c);
                self.edit_cursor += c.len_utf8();
                None
            }
            KeyCode::Tab => {
                self.save_edit();
                self.column = self.column.next();
                self.start_edit();
                None
            }
            KeyCode::BackTab => {
                self.save_edit();
                self.column = self.column.prev();
                self.start_edit();
                None
            }
            _ => None,
        }
    }

    fn handle_confirm_delete_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(idx) = self.confirm_delete.take() {
                    if let Some(note) = self.notes.get(idx).cloned() {
                        let _ = self.store.delete_note(&note.id);
                        self.reload();
                        if self.cursor > 0 && self.cursor >= self.notes.len() {
                            self.cursor = self.notes.len().saturating_sub(1);
                        }
                    }
                }
                Some(AppMessage::Reload)
            }
            _ => {
                self.confirm_delete = None;
                None
            }
        }
    }

    // ── Drawing ──────────────────────────────────────────────────

    fn draw_list(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let col_tags_w: u16 = 14;
        let col_modified_w: u16 = 12;
        let col_title_w = area
            .width
            .saturating_sub(col_tags_w + col_modified_w + 2 + 2 + 2); // +2 for body icon + 2 for pin icon

        // Header line — show sort arrow next to the active sort column
        let sort_arrow = |col: Column, base: &str, width: usize| -> String {
            if self.sort_column == Some(col) {
                let arrow = match self.sort_direction {
                    Some(SortDirection::Ascending) => " \u{2191}",  // ↑
                    Some(SortDirection::Descending) => " \u{2193}", // ↓
                    None => "",
                };
                let label = format!("{}{}", base, arrow);
                pad_right(&label, width)
            } else {
                pad_right(base, width)
            }
        };

        let header = Line::from(vec![
            Span::styled("  ", theme.column_header), // pin prefix spacer
            Span::styled("  ", theme.column_header), // body icon spacer
            Span::styled(
                sort_arrow(Column::Title, "TITLE", col_title_w as usize),
                theme.column_header,
            ),
            Span::styled(
                sort_arrow(Column::Tags, "TAGS", col_tags_w as usize),
                theme.column_header,
            ),
            Span::styled(
                sort_arrow(Column::Modified, "MODIFIED", col_modified_w as usize),
                theme.column_header,
            ),
        ]);

        // Compute visible rows
        let visible_rows = area.height.saturating_sub(1) as usize; // 1 for header
        // When creating, the new row occupies slot 0; existing notes shift by 1
        let creating_offset = if self.creating.is_some() { 1 } else { 0 };
        let scroll = if self.cursor >= visible_rows {
            self.cursor - visible_rows + 1
        } else {
            0
        };

        let mut lines: Vec<Line> = vec![header];

        // ── Inline-creating row ──────────────────────────────────
        if let Some(ref _new_note) = self.creating {
            if scroll == 0 && lines.len() <= visible_rows {
                let before = &self.edit_buf[..self.edit_cursor];
                let after = &self.edit_buf[self.edit_cursor..];
                let cursor_str = format!("{}▏{}", before, after);
                let spans = vec![
                    Span::styled("  ", theme.dim),                    // pin prefix
                    Span::styled("  ", theme.dim),                    // body icon
                    Span::styled(
                        pad_right(&cursor_str, col_title_w as usize),
                        theme.column_focus,
                    ),
                    Span::styled(pad_right("", col_tags_w as usize), theme.dim),
                    Span::styled(pad_right("", col_modified_w as usize), theme.dim),
                ];
                let line = Line::from(spans).style(theme.row_gray);
                lines.push(line);
            }
        }

        for (i, note) in self.notes.iter().enumerate().skip(scroll.saturating_sub(creating_offset)).take(visible_rows.saturating_sub(creating_offset)) {
            let is_selected = i + creating_offset == self.cursor;
            let is_private = note.private || note.title.contains("[p]");

            // Private indicator
            let _is_private_display = is_private && !self.revealed.contains(&note.id);

            // Title
            let title_text = if is_private && !self.revealed.contains(&note.id) {
                mask_private(&note.title, col_title_w as usize)
            } else if self.editing && is_selected && self.column == Column::Title {
                let before = &self.edit_buf[..self.edit_cursor];
                let after = &self.edit_buf[self.edit_cursor..];
                format!("{}|{}", before, after)
            } else {
                truncate(&note.title, col_title_w as usize)
            };

            // Tags
            let tags_text = if self.editing && is_selected && self.column == Column::Tags {
                let before = &self.edit_buf[..self.edit_cursor];
                let after = &self.edit_buf[self.edit_cursor..];
                format!("{}|{}", before, after)
            } else {
                String::new() // placeholder, we use annotated spans below
            };

            // Modified date
            let modified = crate::util::date_format::format_utc_date(&note.updated_at);

            // Build spans
            let mut spans: Vec<Span> = Vec::new();

            // Pin / archive prefix (outside column area)
            let (prefix, prefix_style) = if note.archived {
                (format!("{} ", icons::ARCHIVE), theme.dim)
            } else if note.pinned {
                (format!("{} ", icons::PIN), theme.error)
            } else {
                ("  ".to_string(), theme.dim)
            };
            spans.push(Span::styled(prefix, prefix_style));

            // Body-has-content icon
            let has_body = !note.body.trim().is_empty();
            let body_icon = if has_body { format!("{} ", icons::BODY) } else { "  ".to_string() };
            spans.push(Span::styled(body_icon, theme.dim));

            // Title span
            let title_style = if is_selected && self.column == Column::Title {
                if self.editing {
                    theme.column_focus
                } else {
                    theme.column_focus
                }
            } else if is_private && !self.revealed.contains(&note.id) {
                theme.private
            } else if note.archived {
                theme.dim
            } else {
                theme.title.remove_modifier(Modifier::BOLD)
            };
            spans.push(Span::styled(
                pad_right(&title_text, col_title_w as usize),
                title_style,
            ));

            // Tags span
            if self.editing && is_selected && self.column == Column::Tags {
                spans.push(Span::styled(
                    pad_right(&tags_text, col_tags_w as usize),
                    theme.column_focus,
                ));
            } else if is_selected && self.column == Column::Tags {
                let tag_spans = Self::annotated_tags(note, theme);
                if tag_spans.is_empty() {
                    spans.push(Span::styled(
                        pad_right("", col_tags_w as usize),
                        theme.column_focus,
                    ));
                } else {
                    // Render tags with annotation + padding
                    let mut tags_str = String::new();
                    for t in &note.refs.topics {
                        if !tags_str.is_empty() {
                            tags_str.push(' ');
                        }
                        tags_str.push_str(&format!("#{}", t));
                    }
                    spans.push(Span::styled(
                        pad_right(&truncate(&tags_str, col_tags_w as usize), col_tags_w as usize),
                        theme.column_focus,
                    ));
                }
            } else {
                // Build annotated tag spans
                let mut tag_parts: Vec<Span> = Vec::new();
                let mut tag_len = 0usize;
                for t in &note.refs.topics {
                    let s = format!("#{}", t);
                    tag_len += s.len() + 1;
                    if !tag_parts.is_empty() {
                        tag_parts.push(Span::raw(" "));
                    }
                    tag_parts.push(Span::styled(s, theme.topic));
                }
                // Pad remainder
                let padding = (col_tags_w as usize).saturating_sub(tag_len);
                if tag_parts.is_empty() {
                    spans.push(Span::raw(pad_right("", col_tags_w as usize)));
                } else {
                    spans.extend(tag_parts);
                    if padding > 0 {
                        spans.push(Span::raw(" ".repeat(padding)));
                    }
                }
            }

            // Modified span
            spans.push(Span::styled(
                pad_right(&modified, col_modified_w as usize),
                theme.date,
            ));

            let mut line = Line::from(spans);
            if is_selected {
                line = line.style(theme.row_gray);
            }
            lines.push(line);
        }

        let para = Paragraph::new(lines);
        frame.render_widget(para, area);
    }

    fn draw_preview(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let sep = "\u{2500}".repeat(area.width as usize);
        let sep_line = Line::from(Span::styled(sep, theme.border));

        if let Some(note) = self.current_note() {
            let is_private = note.private || note.title.contains("[p]");
            if is_private && !self.revealed.contains(&note.id) {
                let lines = vec![
                    sep_line,
                    Line::from(Span::styled(
                        " Preview hidden (private note)",
                        theme.private,
                    )),
                ];
                frame.render_widget(Paragraph::new(lines), area);
                return;
            }

            let body_text = if note.body.is_empty() {
                "(empty)"
            } else {
                &note.body
            };

            let max_lines = area.height.saturating_sub(1) as usize;
            let mut lines = vec![sep_line];

            for (i, text_line) in body_text.lines().enumerate() {
                if i >= max_lines {
                    break;
                }
                let trunc = truncate(text_line, area.width.saturating_sub(2) as usize);
                lines.push(Line::from(Span::styled(
                    format!(" {}", trunc),
                    theme.dim,
                )));
            }

            frame.render_widget(Paragraph::new(lines), area);
        } else {
            let lines = vec![
                sep_line,
                Line::from(Span::styled(" No note selected", theme.dim)),
            ];
            frame.render_widget(Paragraph::new(lines), area);
        }
    }

    fn draw_confirm_bar(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if let Some(idx) = self.confirm_delete {
            if let Some(note) = self.notes.get(idx) {
                let title = truncate(&note.title, 30);
                let spans = vec![
                    Span::styled(format!("Delete \"{}\"? ", title), theme.warning),
                    Span::styled("(y/n)", theme.dim),
                ];
                let line = Line::from(spans);
                frame.render_widget(Paragraph::new(line), area);
            }
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────

fn pad_right(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.chars().take(width).collect()
    } else {
        format!("{}{}", s, " ".repeat(width - len))
    }
}
