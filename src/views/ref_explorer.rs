use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::message::AppMessage;
use crate::app::theme::Theme;
use crate::domain::{EntityKind, TaskStatus};
use crate::store::Store;
use crate::util::date_format;

use super::{icons, truncate, render_hint_bar, View};

// ── Entry direction ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryDirection {
    Backlink,
    Outgoing,
}

// ── RefEntry ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct RefEntry {
    kind: EntityKind,
    id: String,
    title: String,
    direction: EntryDirection,
    /// Task status (tasks only).
    status: Option<TaskStatus>,
    /// Formatted tags string, e.g. "#foo #bar". For agendas: "@person".
    tags: String,
    /// Short date string: created date for tasks/notes, meeting date for agendas.
    meta: String,
    /// Total cross-reference count.
    ref_count: usize,
}

// ── RefExplorerOverlay ────────────────────────────────────────────

pub struct RefExplorerOverlay {
    store: Arc<dyn Store>,
    active: bool,
    current_kind: EntityKind,
    current_id: String,
    current_title: String,
    /// Navigation history: (kind, id, title) for back navigation.
    history: Vec<(EntityKind, String, String)>,
    /// Flat list: backlinks first, then outgoing.
    items: Vec<RefEntry>,
    cursor: usize,
}

impl RefExplorerOverlay {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self {
            store,
            active: false,
            current_kind: EntityKind::Task,
            current_id: String::new(),
            current_title: String::new(),
            history: Vec::new(),
            items: Vec::new(),
            cursor: 0,
        }
    }

    /// Open the explorer for a given entity. Clears history and loads refs.
    pub fn open(&mut self, kind: EntityKind, id: String, title: String) {
        self.active = true;
        self.current_kind = kind;
        self.current_id = id;
        self.current_title = title;
        self.history.clear();
        self.load_current();
    }

    #[allow(dead_code)]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Reload items for the current entity.
    fn load_current(&mut self) {
        self.items.clear();
        self.cursor = 0;

        let kind_str = kind_to_str(&self.current_kind);

        // ── Backlinks ──────────────────────────────────────────────
        let backlinks = self.store.get_backlinks(kind_str, &self.current_id);
        for eref in backlinks {
            if let Some(entry) = resolve_entity(&*self.store, &eref.kind, &eref.id, EntryDirection::Backlink) {
                self.items.push(entry);
            }
        }

        // ── Outgoing refs ──────────────────────────────────────────
        let outgoing_ids = match &self.current_kind {
            EntityKind::Task => {
                self.store.get_task(&self.current_id).ok().map(|t| {
                    let mut ids = Vec::new();
                    for id in &t.refs.tasks   { ids.push((EntityKind::Task,   id.clone())); }
                    for id in &t.refs.notes   { ids.push((EntityKind::Note,   id.clone())); }
                    for id in &t.refs.agendas { ids.push((EntityKind::Agenda, id.clone())); }
                    ids
                })
            }
            EntityKind::Note => {
                self.store.get_note(&self.current_id).ok().map(|n| {
                    let mut ids = Vec::new();
                    for id in &n.refs.tasks   { ids.push((EntityKind::Task,   id.clone())); }
                    for id in &n.refs.notes   { ids.push((EntityKind::Note,   id.clone())); }
                    for id in &n.refs.agendas { ids.push((EntityKind::Agenda, id.clone())); }
                    ids
                })
            }
            EntityKind::Agenda => {
                self.store.get_agenda(&self.current_id).ok().map(|a| {
                    let mut ids = Vec::new();
                    for id in &a.refs.tasks   { ids.push((EntityKind::Task,   id.clone())); }
                    for id in &a.refs.notes   { ids.push((EntityKind::Note,   id.clone())); }
                    for id in &a.refs.agendas { ids.push((EntityKind::Agenda, id.clone())); }
                    ids
                })
            }
            _ => None,
        };

        if let Some(ids) = outgoing_ids {
            for (kind, id) in ids {
                if let Some(entry) = resolve_entity(&*self.store, &kind, &id, EntryDirection::Outgoing) {
                    self.items.push(entry);
                }
            }
        }
    }

    /// Navigate into an item: push current to history, set item as current.
    fn drill_into(&mut self, idx: usize) {
        if let Some(item) = self.items.get(idx).cloned() {
            self.history.push((
                self.current_kind.clone(),
                self.current_id.clone(),
                self.current_title.clone(),
            ));
            self.current_kind = item.kind;
            self.current_id = item.id;
            self.current_title = item.title;
            self.load_current();
        }
    }

    /// Navigate back: pop history, restore previous entity.
    fn go_back(&mut self) -> Option<AppMessage> {
        if let Some((kind, id, title)) = self.history.pop() {
            self.current_kind = kind;
            self.current_id = id;
            self.current_title = title;
            self.load_current();
            None
        } else {
            self.active = false;
            Some(AppMessage::CloseRefExplorer)
        }
    }

    // ── Section counts ─────────────────────────────────────────────

    fn backlink_count(&self) -> usize {
        self.items.iter().filter(|e| e.direction == EntryDirection::Backlink).count()
    }

    fn outgoing_count(&self) -> usize {
        self.items.iter().filter(|e| e.direction == EntryDirection::Outgoing).count()
    }
}

// ── View trait ────────────────────────────────────────────────────

impl View for RefExplorerOverlay {
    fn handle_event(&mut self, event: &Event) -> Option<AppMessage> {
        let Event::Key(KeyEvent { code, .. }) = event else {
            return None;
        };

        match code {
            KeyCode::Esc => {
                self.active = false;
                return Some(AppMessage::CloseRefExplorer);
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.items.is_empty() && self.cursor + 1 < self.items.len() {
                    self.cursor += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                if !self.items.is_empty() {
                    let idx = self.cursor;
                    self.drill_into(idx);
                }
            }
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => {
                return self.go_back();
            }
            KeyCode::Char('e') => {
                let id = self.current_id.clone();
                self.active = false;
                let msg = match self.current_kind {
                    EntityKind::Task   => AppMessage::OpenTaskEditor(id),
                    EntityKind::Note   => AppMessage::OpenNoteEditor(id),
                    EntityKind::Agenda => AppMessage::OpenInlineEditor {
                        kind: EntityKind::Agenda,
                        id,
                    },
                    _ => return Some(AppMessage::CloseRefExplorer),
                };
                return Some(msg);
            }
            _ => {}
        }

        None
    }

    fn handle_message(&mut self, _msg: &AppMessage) {}

    fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        // Centered popup: ~70% width, ~60% height, min 50×12
        let overlay_w = ((area.width as u32 * 70 / 100).max(50).min(area.width as u32)) as u16;
        let overlay_h = ((area.height as u32 * 60 / 100).max(12).min(area.height as u32)) as u16;
        let x = area.x + (area.width.saturating_sub(overlay_w)) / 2;
        let y = area.y + (area.height.saturating_sub(overlay_h)) / 2;
        let rect = Rect::new(x, y, overlay_w, overlay_h);

        frame.render_widget(Clear, rect);

        let block = Block::default()
            .title(" Refs ")
            .title_style(theme.title)
            .borders(Borders::ALL)
            .border_style(theme.border)
            .style(theme.popup_bg);

        let inner = block.inner(rect);
        frame.render_widget(block, rect);

        if inner.height < 4 || inner.width < 10 {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // entity header
                Constraint::Length(1), // separator
                Constraint::Min(1),    // items list
                Constraint::Length(1), // hint bar
            ])
            .split(inner);

        self.render_header(frame, chunks[0], theme);

        let sep = "\u{2500}".repeat(chunks[1].width as usize);
        frame.render_widget(
            Paragraph::new(Span::styled(sep, theme.border)),
            chunks[1],
        );

        self.render_items(frame, chunks[2], theme);

        render_hint_bar(frame, chunks[3], &[
            ("\u{2191}\u{2193}", "navigate"),
            ("Enter/\u{2192}", "explore"),
            ("e", "edit"),
            ("\u{2190}", "back"),
            ("Esc", "close"),
        ], theme);
    }

    fn captures_input(&self) -> bool {
        true
    }
}

impl RefExplorerOverlay {
    fn render_header(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let kind_icon = kind_icon(&self.current_kind);
        let kind_label = kind_label(&self.current_kind);
        let title_avail = area.width.saturating_sub(30) as usize;
        let title = truncate(&self.current_title, title_avail.max(10));

        let mut spans = Vec::new();
        spans.push(Span::styled(
            format!("  {} {}  {}  ", kind_icon, kind_label, title),
            theme.title,
        ));

        if let Some((_, _, prev_title)) = self.history.last() {
            let prev = truncate(prev_title, 20);
            spans.push(Span::styled(
                format!("  \u{2190} {}  ", prev),
                theme.dim,
            ));
        }

        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn render_items(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let max_rows = area.height as usize;
        let n_back = self.backlink_count();
        let n_out = self.outgoing_count();

        // Fixed column widths for the metadata section (right side).
        const COL_STATUS_W: usize = 14; // "● In Progress "
        const COL_TAGS_W: usize = 14;   // "#tag1 #tag2   "
        const COL_META_W: usize = 8;    // "Apr 04  "
        const COL_REFS_W: usize = 4;    // "󰌷3   "
        // prefix: "  " (indent=2) + selector "▸ " (2) + icon (1) + " " (1) = 6
        const PREFIX_W: usize = 6;
        let meta_cols_w = COL_STATUS_W + COL_TAGS_W + COL_META_W + COL_REFS_W;

        // Build virtual rows: section header + items for each section.
        // A "virtual row" is either a section header or an item index.
        #[derive(Clone)]
        enum VRow {
            Header(&'static str),
            Item(usize),    // index into self.items
            Empty(&'static str),
        }

        let mut rows: Vec<VRow> = Vec::new();

        rows.push(VRow::Header("  \u{2191} PARENTS"));
        if n_back == 0 {
            rows.push(VRow::Empty("  (no incoming links)"));
        } else {
            for (i, item) in self.items.iter().enumerate() {
                if item.direction == EntryDirection::Backlink {
                    rows.push(VRow::Item(i));
                }
            }
        }

        rows.push(VRow::Header("  \u{2193} CHILDREN"));
        if n_out == 0 {
            rows.push(VRow::Empty("  (no outgoing links)"));
        } else {
            for (i, item) in self.items.iter().enumerate() {
                if item.direction == EntryDirection::Outgoing {
                    rows.push(VRow::Item(i));
                }
            }
        }

        // Find the visual row index of the cursor item.
        let cursor_row = rows.iter().position(|r| matches!(r, VRow::Item(i) if *i == self.cursor))
            .unwrap_or(0);

        let scroll_offset = if cursor_row >= max_rows {
            cursor_row - max_rows + 1
        } else {
            0
        };

        let lines: Vec<Line> = rows.iter()
            .skip(scroll_offset)
            .take(max_rows)
            .map(|row| {
                match row {
                    VRow::Header(label) => {
                        Line::from(Span::styled(*label, theme.column_header))
                    }
                    VRow::Empty(label) => {
                        Line::from(Span::styled(*label, theme.dim))
                    }
                    VRow::Item(idx) => {
                        let item = &self.items[*idx];
                        let is_selected = *idx == self.cursor;

                        let row_style = if is_selected {
                            theme.selected_overlay
                        } else {
                            ratatui::style::Style::default()
                        };

                        let selector = if is_selected { "\u{25b8} " } else { "  " };
                        let icon = kind_icon(&item.kind);

                        let title_w = (area.width as usize)
                            .saturating_sub(PREFIX_W + meta_cols_w)
                            .max(6);
                        let title_padded = pad_right(&truncate(&item.title, title_w), title_w);

                        // Status column (tasks only)
                        let (status_text, status_style) = if let Some(ref s) = item.status {
                            let t = format!("{} {}", s.icon(), s.label());
                            (pad_right(&t, COL_STATUS_W), theme.status_fg(s))
                        } else {
                            (pad_right("", COL_STATUS_W), theme.dim)
                        };

                        // Tags column
                        let tags_padded = pad_right(&truncate(&item.tags, COL_TAGS_W), COL_TAGS_W);

                        // Meta (date) column
                        let meta_padded = pad_right(&item.meta, COL_META_W);

                        // Refs column
                        let refs_text = if item.ref_count > 0 {
                            format!("\u{f0337}{}", item.ref_count)
                        } else {
                            String::new()
                        };
                        let refs_padded = pad_right(&refs_text, COL_REFS_W);
                        let refs_style = if item.ref_count > 0 { theme.accent } else { theme.dim };

                        let title_style = if is_selected { theme.title } else {
                            theme.title.remove_modifier(ratatui::style::Modifier::BOLD)
                        };

                        Line::from(vec![
                            Span::styled(format!("  {}", selector), row_style),
                            Span::styled(format!("{} ", icon), theme.dim),
                            Span::styled(title_padded, title_style),
                            Span::styled(status_text, status_style),
                            Span::styled(tags_padded, theme.topic),
                            Span::styled(meta_padded, theme.date),
                            Span::styled(refs_padded, refs_style),
                        ]).style(row_style)
                    }
                }
            })
            .collect();

        frame.render_widget(Paragraph::new(lines), area);
    }
}

// ── Helpers ───────────────────────────────────────────────────────

fn pad_right(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.chars().take(width).collect()
    } else {
        format!("{}{}", s, " ".repeat(width - len))
    }
}

fn kind_to_str(kind: &EntityKind) -> &'static str {
    match kind {
        EntityKind::Task   => "task",
        EntityKind::Note   => "note",
        EntityKind::Agenda => "agenda",
        EntityKind::Person => "person",
        EntityKind::Tag    => "tag",
    }
}

fn kind_icon(kind: &EntityKind) -> &'static str {
    match kind {
        EntityKind::Task   => icons::TASK,
        EntityKind::Note   => icons::NOTE,
        EntityKind::Agenda => icons::AGENDA,
        EntityKind::Person => icons::MEMORY,
        EntityKind::Tag    => icons::TAG,
    }
}

fn kind_label(kind: &EntityKind) -> &'static str {
    match kind {
        EntityKind::Task   => "TASK",
        EntityKind::Note   => "NOTE",
        EntityKind::Agenda => "AGENDA",
        EntityKind::Person => "PERSON",
        EntityKind::Tag    => "TAG",
    }
}

/// Resolve an entity to a `RefEntry` for display. Returns None if not found.
fn resolve_entity(store: &dyn Store, kind: &EntityKind, id: &str, direction: EntryDirection) -> Option<RefEntry> {
    match kind {
        EntityKind::Task => {
            let task = store.get_task(id).ok()?;
            let title = if task.private { "********".to_string() } else { task.title.clone() };
            let tags = task.refs.tags.iter()
                .map(|t| format!("#{}", t))
                .collect::<Vec<_>>()
                .join(" ");
            let meta = date_format::format_utc_date(&task.created_at);
            let ref_count = task.refs.tasks.len() + task.refs.notes.len() + task.refs.agendas.len();
            Some(RefEntry { kind: kind.clone(), id: id.to_string(), title, direction,
                status: Some(task.status), tags, meta, ref_count })
        }
        EntityKind::Note => {
            let note = store.get_note(id).ok()?;
            let title = if note.private { "********".to_string() } else { note.title.clone() };
            let tags = note.refs.tags.iter()
                .map(|t| format!("#{}", t))
                .collect::<Vec<_>>()
                .join(" ");
            let meta = date_format::format_utc_date(&note.created_at);
            let ref_count = note.refs.tasks.len() + note.refs.notes.len() + note.refs.agendas.len();
            Some(RefEntry { kind: kind.clone(), id: id.to_string(), title, direction,
                status: None, tags, meta, ref_count })
        }
        EntityKind::Agenda => {
            let agenda = store.get_agenda(id).ok()?;
            let tags = format!("@{}", agenda.person_slug);
            let meta = date_format::format_date(&agenda.date);
            let ref_count = agenda.refs.tasks.len() + agenda.refs.notes.len() + agenda.refs.agendas.len();
            Some(RefEntry { kind: kind.clone(), id: id.to_string(), title: agenda.title.clone(),
                direction, status: None, tags, meta, ref_count })
        }
        _ => None,
    }
}
