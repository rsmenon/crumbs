use std::collections::HashSet;
use std::sync::Arc;

use chrono::{Local, Utc};
use crossterm::event::{Event, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::message::AppMessage;
use crate::app::theme::Theme;
use crate::domain::{new_id, Priority, Task, TaskStatus};
use crate::parser::{extract_mentions, extract_topics, parse_datetime};
use crate::store::Store;
use super::{detect_private, icons, mask_private, truncate, View};

// ── Constants ─────────────────────────────────────────────────────

const COL_STATUS_W: u16 = 10;
const COL_PRIORITY_W: u16 = 8;
const COL_TAGS_W: u16 = 14;
const COL_CREATED_W: u16 = 12;
const COL_DUE_W: u16 = 10;

/// Priority levels for tasks.
const PRIORITY_OPTIONS: &[Priority] = &Priority::OPTIONS;

/// Status options for the selector popup (display labels, matched via from_str_loose).
const STATUS_OPTIONS: &[&str] = &["Backlog", "Todo", "Doing", "Blocked", "Done"];

// ── Column enum ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Status = 0,
    Priority = 1,
    Title = 2,
    Tags = 3,
    Created = 4,
    Due = 5,
}

impl Column {
    fn from_index(i: usize) -> Self {
        match i {
            0 => Column::Status,
            1 => Column::Priority,
            2 => Column::Title,
            3 => Column::Tags,
            4 => Column::Created,
            5 => Column::Due,
            _ => Column::Status,
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    fn next(self) -> Self {
        Column::from_index((self.index() + 1).min(5))
    }

    fn prev(self) -> Self {
        if self.index() == 0 {
            Column::Status
        } else {
            Column::from_index(self.index() - 1)
        }
    }
}

// ── Mode ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Creating,
    Editing,
    Filtering,
    ConfirmDelete,
    Selecting, // option popup for Status / Priority
}

// ── Sort direction ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortDirection {
    Ascending,
    Descending,
}

// ── TaskListView ──────────────────────────────────────────────────

pub struct TaskListView {
    store: Arc<dyn Store>,

    // Data
    all: Vec<Task>,
    visible: Vec<Task>,

    // Navigation
    cursor: usize,
    col_cursor: Column,

    // Mode
    mode: Mode,

    // Editing
    edit_idx: usize,
    edit_col: Column,

    // Text input (shared across create/edit/filter)
    input: String,
    input_cursor: usize,

    // Visibility toggles
    show_archived: bool,


    // Sort (column-based: None = unsorted / original load order)
    sort_column: Option<Column>,
    sort_direction: Option<SortDirection>,

    // Filter
    filter_str: String,

    // Dimensions
    content_width: u16,
    content_height: u16,

    // Privacy reveals
    revealed: HashSet<String>,

    // Global tag filter
    tag_filter: Option<String>,

    // Option selector popup (Status / Priority)
    select_col: Column,
    select_cursor: usize,

    // Pending new task being titled inline (not yet saved)
    creating: Option<Task>,
}

impl TaskListView {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self {
            store,
            all: Vec::new(),
            visible: Vec::new(),
            cursor: 0,
            col_cursor: Column::Title,
            mode: Mode::Normal,
            edit_idx: 0,
            edit_col: Column::Title,
            input: String::new(),
            input_cursor: 0,
            show_archived: false,
            sort_column: None,
            sort_direction: None,
            filter_str: String::new(),
            content_width: 0,
            content_height: 0,
            revealed: HashSet::new(),
            tag_filter: None,
            select_col: Column::Status,
            select_cursor: 0,
            creating: None,
        }
    }

    // ── Data loading ──────────────────────────────────────────────

    fn reload(&mut self) {
        if let Ok(mut tasks) = self.store.list_tasks() {
            if let Some(ref tag) = self.tag_filter {
                tasks.retain(|t| t.refs.tags.iter().any(|tg| tg == tag));
            }
            self.all = tasks;
        }
        self.apply_filter();
    }

    fn apply_filter(&mut self) {
        let today = Local::now().date_naive();
        let _ = today; // available for future overdue sorting

        self.visible = self
            .all
            .iter()
            .filter(|t| {
                // Visibility toggles
                if t.archived && !self.show_archived {
                    return false;
                }
                // Text filter
                if !self.filter_str.is_empty() {
                    let q = self.filter_str.to_lowercase();
                    let title_match = t.title.to_lowercase().contains(&q);
                    let status_match = t.status.label().to_lowercase().contains(&q);
                    let tag_match = t
                        .refs
                        .tags
                        .iter()
                        .any(|tg| tg.to_lowercase().contains(&q));
                    let person_match = t
                        .refs
                        .people
                        .iter()
                        .any(|p| p.to_lowercase().contains(&q));
                    if !title_match && !status_match && !tag_match && !person_match {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        // Apply sort
        self.sort_visible();

        // Clamp cursor
        if !self.visible.is_empty() && self.cursor >= self.visible.len() {
            self.cursor = self.visible.len() - 1;
        }
    }

    fn sort_visible(&mut self) {
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
            Column::Status => {
                self.visible.sort_by(|a, b| {
                    flip(a.status.index().cmp(&b.status.index()))
                });
            }
            Column::Title => {
                self.visible.sort_by(|a, b| {
                    flip(a.title.to_lowercase().cmp(&b.title.to_lowercase()))
                });
            }
            Column::Tags => {
                self.visible.sort_by(|a, b| {
                    let tag_a = a.refs.tags.first().map(|t| t.to_lowercase());
                    let tag_b = b.refs.tags.first().map(|t| t.to_lowercase());
                    let ord = match (&tag_a, &tag_b) {
                        (Some(ta), Some(tb)) => ta.cmp(tb),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    };
                    // no-tags always last regardless of direction
                    if tag_a.is_none() && tag_b.is_some() {
                        return std::cmp::Ordering::Greater;
                    }
                    if tag_a.is_some() && tag_b.is_none() {
                        return std::cmp::Ordering::Less;
                    }
                    flip(ord)
                });
            }
            Column::Created => {
                self.visible.sort_by(|a, b| {
                    flip(a.created_at.cmp(&b.created_at))
                });
            }
            Column::Due => {
                self.visible.sort_by(|a, b| {
                    // no-date always last regardless of direction
                    match (&a.due_date, &b.due_date) {
                        (Some(da), Some(db)) => flip(da.cmp(db)),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    }
                });
            }
            Column::Priority => {
                self.visible.sort_by(|a, b| {
                    // None always last regardless of direction
                    match (a.priority, b.priority) {
                        (Priority::None, Priority::None) => std::cmp::Ordering::Equal,
                        (Priority::None, _) => std::cmp::Ordering::Greater,
                        (_, Priority::None) => std::cmp::Ordering::Less,
                        (pa, pb) => flip(pa.cmp(&pb)),
                    }
                });
            }
        }
    }

    fn current_task(&self) -> Option<&Task> {
        self.visible.get(self.cursor)
    }

    // ── Mutations ─────────────────────────────────────────────────

    fn cycle_status(&mut self) {
        if let Some(task) = self.visible.get(self.cursor).cloned() {
            let old_status = task.status;
            let new_status = old_status.next();
            let mut updated = task;
            updated.status = new_status;
            updated.updated_at = Utc::now();

            let _ = self.store.save_task(&updated);
            self.reload();
        }
    }

    fn cycle_priority(&mut self) {
        if let Some(task) = self.visible.get(self.cursor).cloned() {
            let mut updated = task;
            updated.priority = updated.priority.next();
            updated.updated_at = Utc::now();
            let _ = self.store.save_task(&updated);
            self.reload();
        }
    }


    fn save_inline_edit(&mut self) {
        if self.edit_idx >= self.visible.len() {
            return;
        }

        let mut task = self.visible[self.edit_idx].clone();
        let val = self.input.trim().to_string();

        match self.edit_col {
            Column::Status => {
                // Status is cycled with Space, not edited inline — no-op
            }
            Column::Title => {
                let (clean, is_private) = detect_private(&val);
                task.title = clean;
                task.private = is_private;
                task.refs.people = extract_mentions(&task.title);
                task.refs.tags = extract_topics(&task.title);
            }
            Column::Tags => {
                // Parse as space-separated tags (with or without #)
                let tags: Vec<String> = val
                    .split_whitespace()
                    .map(|s| s.trim_start_matches('#').to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect();
                task.refs.tags = tags;
            }
            Column::Created => {
                // Created is read-only — no-op
            }
            Column::Due => {
                if val.is_empty() || val == "none" {
                    task.due_date = None;
                    task.due_time = None;
                } else {
                    let today = Local::now().date_naive();
                    if let Some((d, t, _)) = parse_datetime(&val, today) {
                        task.due_date = Some(d);
                        task.due_time = t.map(|tm| tm.format("%H:%M").to_string());
                    }
                }
            }
            Column::Priority => {
                task.priority = match val.to_lowercase().as_str() {
                    "low" => Priority::Low,
                    "medium" | "med" => Priority::Medium,
                    "high" => Priority::High,
                    _ => Priority::None,
                };
            }
        }

        task.updated_at = Utc::now();
        let _ = self.store.save_task(&task);
        self.reload();
    }

    fn delete_current(&mut self) {
        if let Some(task) = self.current_task().cloned() {
            let _ = self.store.delete_task(&task.id);
            self.reload();
        }
    }


    // ── Input helpers ─────────────────────────────────────────────

    fn start_editing(&mut self) {
        if self.visible.is_empty() {
            return;
        }
        self.mode = Mode::Editing;
        self.edit_idx = self.cursor;
        self.edit_col = self.col_cursor;

        let task = &self.visible[self.edit_idx];
        self.input = Self::column_value(task, self.edit_col);
        self.input_cursor = self.input.len();
    }

    fn column_value(task: &Task, col: Column) -> String {
        match col {
            Column::Status => task.status.label().to_string(),
            Column::Priority => task.priority.label().to_string(),
            Column::Title => task.title.clone(),
            Column::Tags => {
                task.refs
                    .tags
                    .iter()
                    .map(|t| format!("#{}", t))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            Column::Created => crate::util::date_format::format_utc_date(&task.created_at),
            Column::Due => task.due_date.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default(),
        }
    }

    fn start_creating(&mut self) {
        let now = Utc::now();
        let cwd = std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        let task = Task {
            id: new_id(),
            title: String::new(),
            description: String::new(),
            status: TaskStatus::Todo,
            created_at: now,
            updated_at: now,
            due_date: None,
            due_time: None,
            priority: Priority::None,
            private: false,
            pinned: false,
            archived: false,
            created_dir: cwd,
            refs: crate::domain::Refs::default(),
            status_history: Vec::new(),
        };
        self.creating = Some(task);
        self.cursor = 0;
        self.col_cursor = Column::Title;
        self.mode = Mode::Creating;
        self.input.clear();
        self.input_cursor = 0;
    }

    fn start_filtering(&mut self) {
        self.mode = Mode::Filtering;
        self.input = self.filter_str.clone();
        self.input_cursor = self.input.len();
    }

    fn cancel_input(&mut self) {
        self.mode = Mode::Normal;
        self.input.clear();
        self.input_cursor = 0;
    }

    fn handle_input_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(c) => {
                self.input.insert(self.input_cursor, c);
                self.input_cursor += c.len_utf8();
                true
            }
            KeyCode::Backspace => {
                if self.input_cursor > 0 {
                    // Find the previous character boundary
                    let mut new_pos = self.input_cursor - 1;
                    while new_pos > 0 && !self.input.is_char_boundary(new_pos) {
                        new_pos -= 1;
                    }
                    self.input.remove(new_pos);
                    self.input_cursor = new_pos;
                }
                true
            }
            KeyCode::Delete => {
                if self.input_cursor < self.input.len() {
                    self.input.remove(self.input_cursor);
                }
                true
            }
            KeyCode::Left => {
                if self.input_cursor > 0 {
                    self.input_cursor -= 1;
                    while self.input_cursor > 0 && !self.input.is_char_boundary(self.input_cursor) {
                        self.input_cursor -= 1;
                    }
                }
                true
            }
            KeyCode::Right => {
                if self.input_cursor < self.input.len() {
                    self.input_cursor += 1;
                    while self.input_cursor < self.input.len()
                        && !self.input.is_char_boundary(self.input_cursor)
                    {
                        self.input_cursor += 1;
                    }
                }
                true
            }
            KeyCode::Home => {
                self.input_cursor = 0;
                true
            }
            KeyCode::End => {
                self.input_cursor = self.input.len();
                true
            }
            _ => false,
        }
    }
}

// ── View trait impl ───────────────────────────────────────────────

impl View for TaskListView {
    fn handle_event(&mut self, event: &Event) -> Option<AppMessage> {
        let Event::Key(key) = event else {
            return None;
        };

        match self.mode {
            Mode::Creating => return self.handle_creating_key(*key),
            Mode::Editing => return self.handle_editing_key(*key),
            Mode::Filtering => return self.handle_filtering_key(*key),
            Mode::ConfirmDelete => return self.handle_confirm_delete_key(*key),
            Mode::Selecting => return self.handle_selecting_key(*key),
            Mode::Normal => {}
        }

        self.handle_normal_key(*key)
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
            AppMessage::EditEntity { kind: crate::domain::EntityKind::Task, id } => {
                self.reload();
                if let Some(idx) = self.visible.iter().position(|t| t.id == *id) {
                    self.cursor = idx;
                }
            }
            _ => {}
        }
    }

    fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if area.height < 3 || area.width < 20 {
            let p = Paragraph::new("Too small").style(theme.dim);
            frame.render_widget(p, area);
            return;
        }

        // Layout: header + list + optional input line
        let input_lines: u16 = match self.mode {
            Mode::Filtering | Mode::ConfirmDelete => 1,
            Mode::Creating | Mode::Editing | Mode::Normal | Mode::Selecting => 0,
        };

        let constraints = if input_lines > 0 {
            vec![
                Constraint::Length(1), // header
                Constraint::Min(1),    // list
                Constraint::Length(1), // input/status line
            ]
        } else {
            vec![
                Constraint::Length(1), // header
                Constraint::Min(1),    // list
            ]
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        // Render column header
        self.render_header(frame, chunks[0], theme);

        // Render task rows
        self.render_rows(frame, chunks[1], theme);

        // Render input/status line
        if input_lines > 0 && chunks.len() > 2 {
            self.render_input_line(frame, chunks[2], theme);
        }

        // Render selector popup on top when active
        if self.mode == Mode::Selecting {
            self.render_select_popup(frame, area, theme);
        }
    }

    fn captures_input(&self) -> bool {
        self.mode != Mode::Normal
    }
}

impl TaskListView {
    // ── Key handlers by mode ──────────────────────────────────────

    fn handle_normal_key(&mut self, key: KeyEvent) -> Option<AppMessage> {
        match key.code {
            // Navigation
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.visible.is_empty() && self.cursor + 1 < self.visible.len() {
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
                self.col_cursor = self.col_cursor.prev();
                None
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.col_cursor = self.col_cursor.next();
                None
            }
            KeyCode::Char('g') => {
                self.cursor = 0;
                None
            }
            KeyCode::Char('G') => {
                if !self.visible.is_empty() {
                    self.cursor = self.visible.len() - 1;
                }
                None
            }

            // Status / Priority cycle
            KeyCode::Char(' ') => {
                match self.col_cursor {
                    Column::Status => self.cycle_status(),
                    Column::Priority => self.cycle_priority(),
                    _ => self.cycle_status(), // default fallback
                }
                Some(AppMessage::Reload)
            }

            // Inline editing
            KeyCode::Char('e') => {
                self.start_editing();
                None
            }

            // New task
            KeyCode::Char('n') => {
                self.start_creating();
                None
            }

            // Open selector popup for Status/Priority, editor for other columns
            KeyCode::Enter => {
                if let Some(task) = self.current_task() {
                    if task.private && !self.revealed.contains(&task.id) {
                        let id = task.id.clone();
                        self.revealed.insert(id);
                        return None;
                    }
                    match self.col_cursor {
                        Column::Status => {
                            let current = task.status.label();
                            self.select_cursor = STATUS_OPTIONS
                                .iter()
                                .position(|&s| s == current)
                                .unwrap_or(0);
                            self.select_col = Column::Status;
                            self.mode = Mode::Selecting;
                            return None;
                        }
                        Column::Priority => {
                            self.select_cursor = PRIORITY_OPTIONS
                                .iter()
                                .position(|&p| p == task.priority)
                                .unwrap_or(0);
                            self.select_col = Column::Priority;
                            self.mode = Mode::Selecting;
                            return None;
                        }
                        Column::Due => {
                            let current = task.due_date;
                            return Some(AppMessage::OpenDatePicker {
                                date: current,
                                context: crate::app::message::DatePickerContext::TaskDue(
                                    task.id.clone(),
                                ),
                            });
                        }
                        _ => {
                            return Some(AppMessage::OpenTaskEditor(task.id.clone()));
                        }
                    }
                }
                None
            }

            // Delete (with confirmation)
            KeyCode::Char('d') => {
                if self.current_task().is_some() {
                    self.mode = Mode::ConfirmDelete;
                }
                None
            }

            // Filter
            KeyCode::Char('f') => {
                self.start_filtering();
                None
            }

            // Toggle pin
            KeyCode::Char('p') => {
                if let Some(task) = self.visible.get(self.cursor).cloned() {
                    let mut updated = task;
                    updated.pinned = !updated.pinned;
                    updated.updated_at = chrono::Utc::now();
                    let _ = self.store.save_task(&updated);
                    self.reload();
                }
                None
            }

            // Toggle archived flag
            KeyCode::Char('a') => {
                if let Some(task) = self.visible.get(self.cursor).cloned() {
                    let mut updated = task;
                    updated.archived = !updated.archived;
                    if updated.archived {
                        updated.pinned = false;
                    }
                    updated.updated_at = chrono::Utc::now();
                    let _ = self.store.save_task(&updated);
                    self.reload();
                }
                None
            }

            // Toggle archived visibility
            KeyCode::Char('A') => {
                self.show_archived = !self.show_archived;
                self.apply_filter();
                None
            }

            // Column-based sort: asc -> desc -> unsorted
            KeyCode::Char('S') => {
                if self.sort_column == Some(self.col_cursor) {
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
                    self.sort_column = Some(self.col_cursor);
                    self.sort_direction = Some(SortDirection::Ascending);
                }
                self.apply_filter();
                None
            }

            // Clear filter with Esc
            KeyCode::Esc => {
                if !self.filter_str.is_empty() {
                    self.filter_str.clear();
                    self.apply_filter();
                }
                None
            }

            _ => None,
        }
    }

    fn handle_creating_key(&mut self, key: KeyEvent) -> Option<AppMessage> {
        match key.code {
            KeyCode::Enter => {
                let Some(mut task) = self.creating.take() else {
                    self.mode = Mode::Normal;
                    return None;
                };
                let raw = self.input.trim().to_string();
                let title = if raw.is_empty() { "Untitled".to_string() } else { raw };
                let (clean_title, is_private) = detect_private(&title);
                task.title = clean_title;
                task.private = is_private;
                task.refs.people = extract_mentions(&task.title);
                task.refs.tags = extract_topics(&task.title);
                let today = Local::now().date_naive();
                if let Some((d, t, cleaned)) = parse_datetime(&task.title, today) {
                    let final_title = if cleaned.trim().is_empty() {
                        task.title.clone()
                    } else {
                        cleaned.trim().to_string()
                    };
                    task.title = final_title;
                    task.due_date = Some(d);
                    task.due_time = t.map(|tm| tm.format("%H:%M").to_string());
                }
                task.updated_at = Utc::now();
                let id = task.id.clone();
                let _ = self.store.save_task(&task);
                let now_utc = Utc::now();
                for slug in &task.refs.people {
                    if self.store.get_person(slug).is_err() {
                        let person = crate::domain::Person {
                            slug: slug.clone(),
                            created_at: now_utc,
                            pinned: false,
                            archived: false,
                            metadata: Default::default(),
                        };
                        let _ = self.store.save_person(&person);
                    }
                }
                for slug in &task.refs.tags {
                    if self.store.get_tag(slug).is_err() {
                        let tag = crate::domain::Tag {
                            slug: slug.clone(),
                            created_at: now_utc,
                        };
                        let _ = self.store.save_tag(&tag);
                    }
                }
                self.cancel_input();
                self.reload();
                // Select the newly created task
                if let Some(idx) = self.visible.iter().position(|t| t.id == id) {
                    self.cursor = idx;
                }
                Some(AppMessage::Reload)
            }
            KeyCode::Esc => {
                self.creating = None;
                self.cancel_input();
                None
            }
            _ => {
                self.handle_input_key(key);
                None
            }
        }
    }

    fn handle_editing_key(&mut self, key: KeyEvent) -> Option<AppMessage> {
        match key.code {
            KeyCode::Enter => {
                self.save_inline_edit();
                self.mode = Mode::Normal;
                Some(AppMessage::Reload)
            }
            KeyCode::Esc => {
                self.cancel_input();
                None
            }
            KeyCode::Tab => {
                // Save current, move to next column
                self.save_inline_edit();
                self.edit_col = self.edit_col.next();
                self.col_cursor = self.edit_col;
                // Load new column's value
                if self.edit_idx < self.visible.len() {
                    let task = &self.visible[self.edit_idx];
                    self.input = Self::column_value(task, self.edit_col);
                    self.input_cursor = self.input.len();
                }
                None
            }
            KeyCode::BackTab => {
                // Save current, move to prev column
                self.save_inline_edit();
                self.edit_col = self.edit_col.prev();
                self.col_cursor = self.edit_col;
                if self.edit_idx < self.visible.len() {
                    let task = &self.visible[self.edit_idx];
                    self.input = Self::column_value(task, self.edit_col);
                    self.input_cursor = self.input.len();
                }
                None
            }
            _ => {
                self.handle_input_key(key);
                None
            }
        }
    }

    fn handle_filtering_key(&mut self, key: KeyEvent) -> Option<AppMessage> {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                self.filter_str = self.input.clone();
                self.mode = Mode::Normal;
                self.apply_filter();
                None
            }
            _ => {
                self.handle_input_key(key);
                // Live filter as user types
                self.filter_str = self.input.clone();
                self.apply_filter();
                None
            }
        }
    }

    fn handle_confirm_delete_key(&mut self, key: KeyEvent) -> Option<AppMessage> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.delete_current();
                self.mode = Mode::Normal;
                None
            }
            _ => {
                // Any other key cancels
                self.mode = Mode::Normal;
                None
            }
        }
    }

    fn handle_selecting_key(&mut self, key: KeyEvent) -> Option<AppMessage> {
        let n_opts = match self.select_col {
            Column::Status => STATUS_OPTIONS.len(),
            Column::Priority => PRIORITY_OPTIONS.len(),
            _ => { self.mode = Mode::Normal; return None; }
        };
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.select_cursor + 1 < n_opts {
                    self.select_cursor += 1;
                }
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.select_cursor > 0 {
                    self.select_cursor -= 1;
                }
                None
            }
            KeyCode::Enter => {
                let msg = self.apply_selection();
                self.mode = Mode::Normal;
                msg
            }
            _ => None,
        }
    }

    fn apply_selection(&mut self) -> Option<AppMessage> {
        let Some(task) = self.visible.get(self.cursor).cloned() else { return None };
        let mut updated = task;
        match self.select_col {
            Column::Status => {
                let label = STATUS_OPTIONS.get(self.select_cursor).copied().unwrap_or("Backlog");
                if let Some(s) = TaskStatus::from_str_loose(&label.to_lowercase()) {
                    updated.status = s;
                }
            }
            Column::Priority => {
                updated.priority = PRIORITY_OPTIONS.get(self.select_cursor).copied().unwrap_or(Priority::None);
            }
            _ => return None,
        }
        updated.updated_at = chrono::Utc::now();
        let _ = self.store.save_task(&updated);
        self.reload();
        Some(AppMessage::Reload)
    }

    // ── Rendering ─────────────────────────────────────────────────

    fn render_select_popup(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        use ratatui::style::Style;
        use ratatui::widgets::{Block, Borders, Clear};

        let (title, n_opts): (&str, usize) = match self.select_col {
            Column::Status => (" Status ", STATUS_OPTIONS.len()),
            Column::Priority => (" Priority ", PRIORITY_OPTIONS.len()),
            _ => return,
        };

        let popup_w = 24u16.min(area.width);
        let popup_h = (n_opts as u16 + 2).min(area.height); // options + 2 borders

        let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
        let y = area.y + (area.height.saturating_sub(popup_h)) / 3;
        let popup_rect = Rect::new(x, y, popup_w, popup_h);

        frame.render_widget(Clear, popup_rect);

        let block = Block::default()
            .title(title)
            .title_style(theme.title)
            .borders(Borders::ALL)
            .border_style(theme.border)
            .style(theme.popup_bg);

        let inner = block.inner(popup_rect);
        frame.render_widget(block, popup_rect);

        if inner.height == 0 {
            return;
        }

        let lines: Vec<Line<'_>> = (0..n_opts)
            .take(inner.height as usize)
            .map(|i| {
                let is_sel = i == self.select_cursor;
                let prefix = if is_sel { " ▸ " } else { "   " };

                let (label, value_style) = match self.select_col {
                    Column::Status => {
                        let opt = STATUS_OPTIONS[i];
                        let style = if is_sel {
                            theme.selected
                        } else {
                            TaskStatus::from_str_loose(&opt.to_lowercase())
                                .map(|s| theme.status_fg(&s))
                                .unwrap_or(Style::default())
                        };
                        (opt, style)
                    }
                    Column::Priority => {
                        let p = PRIORITY_OPTIONS[i];
                        let label = if p.is_none() { "None" } else { p.label() };
                        let style = if is_sel {
                            theme.selected
                        } else {
                            match p {
                                Priority::High   => theme.priority_high,
                                Priority::Medium => theme.priority_medium,
                                Priority::Low    => theme.priority_low,
                                Priority::None   => theme.dim,
                            }
                        };
                        (label, style)
                    }
                    _ => ("", Style::default()),
                };

                Line::from(vec![
                    Span::styled(prefix, if is_sel { theme.selected } else { theme.dim }),
                    Span::styled(label, value_style),
                ])
            })
            .collect();

        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn render_header(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let title_w = area
            .width
            .saturating_sub(COL_STATUS_W + COL_PRIORITY_W + COL_TAGS_W + COL_CREATED_W + COL_DUE_W + 5 + 2 + 2) as usize; // 5 separators + 2 body icon + 2 pin prefix

        // Helper: append sort arrow to header label if this column is the active sort column
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

        let mut spans = Vec::new();
        spans.push(Span::styled("  ", theme.column_header)); // pin prefix spacer
        spans.push(Span::styled(
            pad_right("STATUS", COL_STATUS_W as usize),
            theme.column_header,
        ));
        spans.push(Span::styled(" ", theme.border));
        spans.push(Span::styled(
            pad_right("PRIORITY", COL_PRIORITY_W as usize),
            theme.column_header,
        ));
        spans.push(Span::styled(" ", theme.border));
        spans.push(Span::styled("  ", theme.column_header)); // body icon spacer
        spans.push(Span::styled(
            sort_arrow(Column::Title, "TITLE", title_w),
            theme.column_header,
        ));
        spans.push(Span::styled(" ", theme.border));
        spans.push(Span::styled(
            sort_arrow(Column::Tags, "TAGS", COL_TAGS_W as usize),
            theme.column_header,
        ));
        spans.push(Span::styled(" ", theme.border));
        spans.push(Span::styled(
            sort_arrow(Column::Created, "CREATED", COL_CREATED_W as usize),
            theme.column_header,
        ));
        spans.push(Span::styled(" ", theme.border));
        spans.push(Span::styled(
            sort_arrow(Column::Due, "DUE", COL_DUE_W as usize),
            theme.column_header,
        ));

        // Filter indicator
        if !self.filter_str.is_empty() {
            spans.push(Span::styled(
                format!(" [filter:{}]", self.filter_str),
                theme.dim,
            ));
        }

        let line = Line::from(spans);
        frame.render_widget(Paragraph::new(line), area);
    }

    fn render_rows(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let max_rows = area.height as usize;

        if self.visible.is_empty() && self.creating.is_none() {
            let msg = if self.filter_str.is_empty() {
                "No tasks. Press 'n' to create one."
            } else {
                "No matching tasks."
            };
            let p = Paragraph::new(Span::styled(msg, theme.dim));
            frame.render_widget(p, area);
            return;
        }

        let creating_offset = if self.creating.is_some() { 1 } else { 0 };

        // Scroll window
        let scroll_offset = if self.cursor >= max_rows {
            self.cursor - max_rows + 1
        } else {
            0
        };

        let title_w = area
            .width
            .saturating_sub(COL_STATUS_W + COL_PRIORITY_W + COL_TAGS_W + COL_CREATED_W + COL_DUE_W + 5 + 2 + 2) as usize; // 5 separators + 2 body icon + 2 pin prefix

        let today = Local::now().date_naive();

        let mut lines: Vec<Line> = Vec::with_capacity(max_rows);

        // ── Inline-creating row ───────────────────────────────────
        if self.creating.is_some() && scroll_offset == 0 && lines.len() < max_rows {
            let before = &self.input[..self.input_cursor];
            let after = &self.input[self.input_cursor..];
            let title_input = pad_right(&format!("{}▏{}", before, after), title_w);
            let empty_status = pad_right("○ Todo", COL_STATUS_W as usize);
            let empty_priority = pad_right("", COL_PRIORITY_W as usize);
            let empty_tags = pad_right("", COL_TAGS_W as usize);
            let empty_created = pad_right("", COL_CREATED_W as usize);
            let empty_due = pad_right("", COL_DUE_W as usize);
            let spans = vec![
                Span::styled("  ", theme.dim),
                Span::styled(empty_status, theme.dim),
                Span::styled(" ", theme.border),
                Span::styled(empty_priority, theme.dim),
                Span::styled(" ", theme.border),
                Span::styled("  ", theme.dim), // body icon spacer
                Span::styled(title_input, theme.column_focus),
                Span::styled(" ", theme.border),
                Span::styled(empty_tags, theme.dim),
                Span::styled(" ", theme.border),
                Span::styled(empty_created, theme.dim),
                Span::styled(" ", theme.border),
                Span::styled(empty_due, theme.dim),
            ];
            lines.push(Line::from(spans).style(theme.row_gray));
        }

        for (vi, task) in self.visible.iter().enumerate().skip(scroll_offset.saturating_sub(creating_offset)).take(max_rows.saturating_sub(creating_offset)) {
            let is_selected = vi + creating_offset == self.cursor;
            let is_done = task.status == TaskStatus::Done;
            let is_archived = task.archived;
            let is_private = task.private && !self.revealed.contains(&task.id);

            // Being edited inline?
            let editing_this_row = self.mode == Mode::Editing && vi == self.edit_idx;

            // ── Status column ─────────────────────────────────
            let status_text = format!(
                "{} {}",
                task.status.icon(),
                task.status.label()
            );
            let status_text = pad_right(&status_text, COL_STATUS_W as usize);
            let status_style = if is_archived {
                theme.dim
            } else {
                theme.status_fg(&task.status)
            };

            // ── Title column ──────────────────────────────────
            let title_display = if is_private {
                mask_private(&task.title, title_w)
            } else {
                truncate(&task.title, title_w)
            };
            let title_padded = pad_right(&title_display, title_w);

            let title_style = if is_private {
                theme.private
            } else if is_archived {
                theme.dim
            } else if is_done {
                theme.status_done
            } else {
                theme.title.remove_modifier(Modifier::BOLD)
            };

            // ── Tags column ───────────────────────────────────
            let tags_text = task
                .refs
                .tags
                .iter()
                .map(|t| format!("#{}", t))
                .collect::<Vec<_>>()
                .join(" ");
            let tags_display = truncate(&tags_text, COL_TAGS_W as usize);
            let tags_padded = pad_right(&tags_display, COL_TAGS_W as usize);
            let tags_style = if is_archived {
                theme.dim
            } else {
                theme.topic
            };

            // ── Due column ────────────────────────────────────
            let due_text = task
                .due_date
                .map(|d| format_short_date(&d))
                .unwrap_or_default();
            let due_padded = pad_right(&due_text, COL_DUE_W as usize);
            let due_date_str = task.due_date.map(|d| d.format("%Y-%m-%d").to_string());
            let due_style = if is_archived || is_done {
                theme.dim
            } else {
                theme.due_date_style(due_date_str.as_deref(), today)
            };

            // ── Created column ────────────────────────────────
            let created_text = crate::util::date_format::format_utc_date(&task.created_at);
            let created_padded = pad_right(&created_text, COL_CREATED_W as usize);
            let created_style = if is_archived {
                theme.dim
            } else {
                theme.date
            };

            // ── Priority column ───────────────────────────────
            let priority_text = task.priority.label();
            let priority_padded = pad_right(priority_text, COL_PRIORITY_W as usize);
            let priority_style = if is_archived {
                theme.dim
            } else {
                match task.priority {
                    Priority::High   => theme.priority_high,
                    Priority::Medium => theme.priority_medium,
                    Priority::Low    => theme.priority_low,
                    Priority::None   => theme.dim,
                }
            };

            // ── Body-has-content icon ────────────────────────
            let has_body = !task.description.trim().is_empty();
            let body_icon = if has_body { format!("{} ", icons::BODY) } else { "  ".to_string() };

            // ── Build spans with column focus ─────────────────
            let mut spans = Vec::new();

            // Pin / archive prefix (before status, outside column area)
            let (pin_prefix, pin_style) = if is_archived {
                (format!("{} ", icons::ARCHIVE), theme.dim)
            } else if task.pinned {
                (format!("{} ", icons::PIN), theme.error)
            } else {
                ("  ".to_string(), theme.dim)
            };
            spans.push(Span::styled(pin_prefix, pin_style));

            // Status column
            if is_selected && self.col_cursor == Column::Status {
                spans.push(Span::styled(status_text, theme.column_focus));
            } else {
                spans.push(Span::styled(status_text, status_style));
            }
            spans.push(Span::styled(" ", theme.border));

            // Priority column
            if editing_this_row && self.edit_col == Column::Priority {
                let before = &self.input[..self.input_cursor];
                let after = &self.input[self.input_cursor..];
                let input_display = pad_right(&format!("{}|{}", before, after), COL_PRIORITY_W as usize);
                spans.push(Span::styled(input_display, theme.column_focus));
            } else if is_selected && self.col_cursor == Column::Priority {
                spans.push(Span::styled(priority_padded, theme.column_focus));
            } else {
                spans.push(Span::styled(priority_padded, priority_style));
            }
            spans.push(Span::styled(" ", theme.border));

            // Body icon before title
            spans.push(Span::styled(body_icon, theme.dim));

            // Title column - check if editing or focused
            if editing_this_row && self.edit_col == Column::Title {
                let before = &self.input[..self.input_cursor];
                let after = &self.input[self.input_cursor..];
                let input_display = pad_right(&format!("{}|{}", before, after), title_w);
                spans.push(Span::styled(input_display, theme.column_focus));
            } else if is_selected && self.col_cursor == Column::Title {
                spans.push(Span::styled(title_padded, theme.column_focus));
            } else {
                spans.push(Span::styled(title_padded, title_style));
            }

            spans.push(Span::styled(" ", theme.border));

            // Tags column
            if editing_this_row && self.edit_col == Column::Tags {
                let before = &self.input[..self.input_cursor];
                let after = &self.input[self.input_cursor..];
                let input_display = pad_right(&format!("{}|{}", before, after), COL_TAGS_W as usize);
                spans.push(Span::styled(input_display, theme.column_focus));
            } else if is_selected && self.col_cursor == Column::Tags {
                spans.push(Span::styled(tags_padded, theme.column_focus));
            } else {
                spans.push(Span::styled(tags_padded, tags_style));
            }

            spans.push(Span::styled(" ", theme.border));

            // Created column
            if editing_this_row && self.edit_col == Column::Created {
                // Read-only: show the date in column_focus style but no real editing
                let input_display = pad_right(&self.input, COL_CREATED_W as usize);
                spans.push(Span::styled(input_display, theme.column_focus));
            } else if is_selected && self.col_cursor == Column::Created {
                spans.push(Span::styled(created_padded, theme.column_focus));
            } else {
                spans.push(Span::styled(created_padded, created_style));
            }

            spans.push(Span::styled(" ", theme.border));

            // Due column
            if editing_this_row && self.edit_col == Column::Due {
                let before = &self.input[..self.input_cursor];
                let after = &self.input[self.input_cursor..];
                let input_display = pad_right(&format!("{}|{}", before, after), COL_DUE_W as usize);
                spans.push(Span::styled(input_display, theme.column_focus));
            } else if is_selected && self.col_cursor == Column::Due {
                spans.push(Span::styled(due_padded, theme.column_focus));
            } else {
                spans.push(Span::styled(due_padded, due_style));
            }

            let mut line = Line::from(spans);

            // Row-level highlight for selected row
            if is_selected && !editing_this_row {
                line = line.style(theme.row_gray);
            }

            lines.push(line);
        }

        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, area);
    }

    fn render_input_line(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        match self.mode {
            Mode::Creating => {} // rendered as an inline row at the top of the list
            Mode::Filtering => {
                let fg = theme.title.remove_modifier(Modifier::BOLD);
                let before = &self.input[..self.input_cursor];
                let after = &self.input[self.input_cursor..];
                let spans = vec![
                    Span::styled("filter: ", theme.dim),
                    Span::styled(before, fg),
                    Span::styled("▏", fg.add_modifier(Modifier::SLOW_BLINK)),
                    Span::styled(after, fg),
                ];
                let line = Line::from(spans);
                frame.render_widget(Paragraph::new(line), area);
            }
            Mode::Editing => {} // rendered inline in the row cell
            Mode::ConfirmDelete => {
                if let Some(task) = self.current_task() {
                    let title = truncate(&task.title, 30);
                    let spans = vec![
                        Span::styled(format!("Delete \"{}\"? ", title), theme.warning),
                        Span::styled("(y/n)", theme.dim),
                    ];
                    let line = Line::from(spans);
                    frame.render_widget(Paragraph::new(line), area);
                }
            }
            Mode::Normal | Mode::Selecting => {}
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────

fn pad_right(s: &str, width: usize) -> String {
    let display_w = display_width(s);
    if display_w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - display_w))
    }
}

/// Approximate display width (counts chars, not graphemes, which is
/// close enough for ASCII-heavy TUI content).
fn display_width(s: &str) -> usize {
    // Use ratatui's unicode width if available, otherwise char count
    s.chars().count()
}


fn format_short_date(date: &chrono::NaiveDate) -> String {
    crate::util::date_format::format_date(date)
}
