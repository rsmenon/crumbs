use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{Datelike, Local, NaiveDate};
use crossterm::event::{Event, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::message::{AppMessage, DatePickerContext};
use crate::app::theme::Theme;
use crate::domain::{Agenda, EntityKind, EntityRef, Note, Priority, Task, TaskStatus};
use crate::store::Store;
use crate::util::calendar::{month_grid, weekday_headers};
use crate::util::date_format::{format_date, format_utc_date};
use super::{icons, mask_private, truncate, View};

// ── Focus state ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CalendarFocus {
    Month,
    Day,
    Details,
}

// ── Detail pane state ─────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailMode {
    Normal,
    EditingText,
    Selecting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailField {
    // shared
    Title,
    Tags,
    Created,
    // task
    Status,
    Priority,
    Due,
    // note
    Modified,
    // agenda
    Date,
    Person,
    // (Done and RemindAt removed with Todo/Reminder entities)
    // shared (shown for tasks + notes)
    People,
    Dir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Text,
    Select,
    DatePick,
    ReadOnly,
}

struct FieldRow {
    field: DetailField,
    label: &'static str,
    value: String,
    kind: FieldKind,
}

#[derive(Debug, Clone)]
enum LoadedEntity {
    Task(Task),
    Note(Note),
    Agenda(Agenda),
}

const STATUS_OPTIONS: &[&str] = &["Backlog", "Todo", "Doing", "Blocked", "Done"];
const PRIORITY_OPTIONS: &[Priority] = &Priority::OPTIONS;
const PRIORITY_LABELS: &[&str] = &["None", "Low", "Medium", "High"];
/// Width of the key name column in the detail pane (matches nvim overlay header).
const KEY_W: usize = 10;
/// Total prefix width: 2 indent + 2 glyph + 2 gap + KEY_W key column.
const DETAIL_PREFIX_W: usize = 2 + 2 + 2 + KEY_W;

// ── Day item resolved from EntityRef ─────────────────────────────

#[derive(Debug, Clone)]
struct DayItem {
    id: String,
    kind: EntityKind,
    title: String,
    time: Option<String>,
    private: bool,
    done: bool,
}

impl DayItem {
    fn icon(&self) -> &'static str {
        match self.kind {
            EntityKind::Task => icons::TASK,
            EntityKind::Note => icons::NOTE,
            EntityKind::Agenda => icons::AGENDA,
            _ => icons::TASK,
        }
    }
}

// ── CalendarView ─────────────────────────────────────────────────

pub struct CalendarView {
    store: Arc<dyn Store>,
    year: i32,
    month: u32,
    grid: [[u8; 7]; 6],
    row: usize,
    col: usize,
    /// Number of events per day (1-31 indexed) for dot indicators.
    event_counts: HashMap<u8, usize>,
    /// The selected date for day panel.
    selected_date: NaiveDate,
    /// Items for the selected date.
    day_items: Vec<DayItem>,
    /// Cursor within day items list.
    day_cursor: usize,
    /// Which pane has keyboard focus.
    focus: CalendarFocus,
    /// Set of revealed private entry ids.
    revealed: HashSet<String>,
    content_width: u16,
    content_height: u16,
    /// Global tag filter.
    tag_filter: Option<String>,
    /// Index of day item awaiting delete confirmation.
    confirm_delete: Option<usize>,

    // ── Task creation prompt ──────────────────────────────────────
    /// True while the inline "new task title" prompt is active.
    creating_task: bool,
    new_task_buf: String,
    new_task_cursor: usize,

    // ── Detail pane ───────────────────────────────────────────────
    /// The fully loaded entity shown in the detail pane.
    detail_entity: Option<LoadedEntity>,
    /// Which field row is highlighted in the detail pane.
    detail_field_cursor: usize,
    /// Whether we are editing/selecting in the detail pane.
    detail_mode: DetailMode,
    /// Text input buffer for inline field editing.
    detail_input: String,
    detail_input_cursor: usize,
    /// Cursor within a selection popup (status / priority).
    detail_select_cursor: usize,
    /// Options shown in the current selection popup.
    detail_select_options: &'static [&'static str],
    /// Which field triggered the current selection popup.
    detail_select_field: DetailField,
}

impl CalendarView {
    pub fn new(store: Arc<dyn Store>) -> Self {
        let today = Local::now().date_naive();
        let year = today.year();
        let month = today.month();
        let grid = month_grid(year, month);

        let (row, col) = find_day_in_grid(&grid, today.day() as u8).unwrap_or((0, 0));

        let mut view = Self {
            store,
            year,
            month,
            grid,
            row,
            col,
            event_counts: HashMap::new(),
            selected_date: today,
            day_items: Vec::new(),
            day_cursor: 0,
            focus: CalendarFocus::Month,
            revealed: HashSet::new(),
            content_width: 80,
            content_height: 24,
            tag_filter: None,
            confirm_delete: None,
            creating_task: false,
            new_task_buf: String::new(),
            new_task_cursor: 0,
            detail_entity: None,
            detail_field_cursor: 0,
            detail_mode: DetailMode::Normal,
            detail_input: String::new(),
            detail_input_cursor: 0,
            detail_select_cursor: 0,
            detail_select_options: &[],
            detail_select_field: DetailField::Title,
        };
        view.refresh_event_counts();
        view.load_day_items();
        view
    }

    // ── Grid navigation ──────────────────────────────────────────

    fn refresh_grid(&mut self) {
        self.grid = month_grid(self.year, self.month);
    }

    fn refresh_event_counts(&mut self) {
        self.event_counts.clear();
        for day in 1..=31u8 {
            let date_str = format!("{}-{:02}-{:02}", self.year, self.month, day);
            let refs = self.store.entities_by_date(&date_str);
            if !refs.is_empty() {
                self.event_counts.insert(day, refs.len());
            }
        }
    }

    fn current_day(&self) -> u8 {
        self.grid[self.row][self.col]
    }

    fn move_up(&mut self) {
        if self.row == 0 {
            return;
        }
        let mut r = self.row - 1;
        while r < 6 {
            if self.grid[r][self.col] != 0 {
                self.row = r;
                return;
            }
            if r == 0 {
                break;
            }
            r -= 1;
        }
    }

    fn move_down(&mut self) {
        if self.row >= 5 {
            return;
        }
        let mut r = self.row + 1;
        while r < 6 {
            if self.grid[r][self.col] != 0 {
                self.row = r;
                return;
            }
            r += 1;
        }
    }

    fn move_left(&mut self) {
        if self.col == 0 {
            // Wrap to the previous row, scanning right-to-left for a valid day
            if self.row == 0 {
                return;
            }
            let prev_row = self.row - 1;
            let mut c = 6;
            loop {
                if self.grid[prev_row][c] != 0 {
                    self.row = prev_row;
                    self.col = c;
                    return;
                }
                if c == 0 {
                    break;
                }
                c -= 1;
            }
            return;
        }
        let mut c = self.col - 1;
        loop {
            if self.grid[self.row][c] != 0 {
                self.col = c;
                return;
            }
            if c == 0 {
                break;
            }
            c -= 1;
        }
    }

    fn move_right(&mut self) {
        let mut c = self.col + 1;
        while c < 7 {
            if self.grid[self.row][c] != 0 {
                self.col = c;
                return;
            }
            c += 1;
        }
        if self.row < 5 && self.grid[self.row + 1][0] != 0 {
            self.row += 1;
            self.col = 0;
        }
    }

    fn go_prev_month(&mut self) {
        if self.month == 1 {
            self.month = 12;
            self.year -= 1;
        } else {
            self.month -= 1;
        }
        self.refresh_grid();
        self.refresh_event_counts();
        self.row = 0;
        self.col = 0;
        self.snap_to_valid();
    }

    fn go_next_month(&mut self) {
        if self.month == 12 {
            self.month = 1;
            self.year += 1;
        } else {
            self.month += 1;
        }
        self.refresh_grid();
        self.refresh_event_counts();
        self.row = 0;
        self.col = 0;
        self.snap_to_valid();
    }

    fn go_today(&mut self) {
        let today = Local::now().date_naive();
        self.year = today.year();
        self.month = today.month();
        self.refresh_grid();
        self.refresh_event_counts();
        if let Some((r, c)) = find_day_in_grid(&self.grid, today.day() as u8) {
            self.row = r;
            self.col = c;
        }
    }

    fn snap_to_valid(&mut self) {
        if self.grid[self.row][self.col] != 0 {
            return;
        }
        for r in 0..6 {
            for c in 0..7 {
                if self.grid[r][c] != 0 {
                    self.row = r;
                    self.col = c;
                    return;
                }
            }
        }
    }

    // ── Day panel helpers ─────────────────────────────────────────

    fn sync_day_from_cursor(&mut self) {
        let day = self.current_day();
        if day == 0 {
            return;
        }
        if let Some(date) = NaiveDate::from_ymd_opt(self.year, self.month, day as u32) {
            self.selected_date = date;
            self.day_cursor = 0;
            self.load_day_items();
            self.load_detail_item();
        }
    }

    fn load_day_items(&mut self) {
        self.day_items.clear();
        let date_str = self.selected_date.format("%Y-%m-%d").to_string();
        let refs = self.store.entities_by_date(&date_str);

        for eref in &refs {
            if let Some(item) = self.resolve_entity_ref(eref) {
                if let Some(ref tag) = self.tag_filter {
                    let has_tag = match eref.kind {
                        EntityKind::Task => {
                            let id = eref.id.clone();
                            self.store.get_task(&id)
                                .map(|t| t.refs.tags.iter().any(|tg| tg == tag))
                                .unwrap_or(false)
                        }
                        EntityKind::Note => {
                            let id = eref.id.clone();
                            self.store.get_note(&id)
                                .map(|n| n.refs.tags.iter().any(|tg| tg == tag))
                                .unwrap_or(false)
                        }
                        _ => false,
                    };
                    if !has_tag {
                        continue;
                    }
                }
                self.day_items.push(item);
            }
        }
    }

    fn resolve_entity_ref(&self, eref: &EntityRef) -> Option<DayItem> {
        match eref.kind {
            EntityKind::Task => {
                let id = eref.id.clone();
                if let Ok(t) = self.store.get_task(&id) {
                    return Some(DayItem {
                        id: t.id.clone(),
                        kind: EntityKind::Task,
                        title: t.title.clone(),
                        time: t.due_time.clone(),
                        private: t.private,
                        done: t.status == TaskStatus::Done,
                    });
                }
            }
            EntityKind::Note => {
                let id = eref.id.clone();
                if let Ok(n) = self.store.get_note(&id) {
                    return Some(DayItem {
                        id: n.id.clone(),
                        kind: EntityKind::Note,
                        title: n.title.clone(),
                        time: None,
                        private: n.private || n.title.contains("[p]"),
                        done: false,
                    });
                }
            }
            EntityKind::Agenda => {
                let id = eref.id.clone();
                if let Ok(a) = self.store.get_agenda(&id) {
                    return Some(DayItem {
                        id: a.id.clone(),
                        kind: EntityKind::Agenda,
                        title: a.title.clone(),
                        time: None,
                        private: false,
                        done: false,
                    });
                }
            }
            _ => {}
        }
        None
    }

    // ── Detail pane helpers ───────────────────────────────────────

    /// Load the entity at `day_cursor` into the detail pane.
    fn load_detail_item(&mut self) {
        self.detail_entity = None;
        self.detail_field_cursor = 0;
        self.detail_mode = DetailMode::Normal;
        let Some(item) = self.day_items.get(self.day_cursor) else { return };
        let entity = match item.kind {
            EntityKind::Task => self.store.get_task(&item.id).ok().map(LoadedEntity::Task),
            EntityKind::Note => self.store.get_note(&item.id).ok().map(LoadedEntity::Note),
            EntityKind::Agenda => self.store.get_agenda(&item.id).ok().map(LoadedEntity::Agenda),
            _ => None,
        };
        self.detail_entity = entity;
    }

    /// Build the list of field rows for the currently loaded entity.
    fn detail_fields(&self) -> Vec<FieldRow> {
        match &self.detail_entity {
            Some(LoadedEntity::Task(t)) => {
                let tags = t.refs.tags.iter()
                    .map(|s| format!("#{}", s))
                    .collect::<Vec<_>>()
                    .join(" ");
                let people = t.refs.people.iter()
                    .map(|s| format!("@{}", s))
                    .collect::<Vec<_>>()
                    .join(" ");
                let mut rows = vec![
                    FieldRow { field: DetailField::Title,    label: "Title",    value: t.title.clone(),                    kind: FieldKind::Text     },
                    FieldRow { field: DetailField::Status,   label: "Status",   value: t.status.label().to_string(),        kind: FieldKind::Select   },
                    FieldRow { field: DetailField::Priority, label: "Priority", value: t.priority.label().to_string(),       kind: FieldKind::Select   },
                    FieldRow { field: DetailField::Due,      label: "Due",      value: t.due_date.as_ref().map(format_date).unwrap_or_default(), kind: FieldKind::DatePick },
                    FieldRow { field: DetailField::Tags,     label: "Tags",     value: tags,                                kind: FieldKind::Text     },
                    FieldRow { field: DetailField::Created,  label: "Created",  value: format_utc_date(&t.created_at),      kind: FieldKind::ReadOnly },
                ];
                if !people.is_empty() {
                    rows.push(FieldRow { field: DetailField::People, label: "People", value: people, kind: FieldKind::ReadOnly });
                }
                if !t.created_dir.is_empty() {
                    rows.push(FieldRow { field: DetailField::Dir, label: "Dir", value: t.created_dir.clone(), kind: FieldKind::ReadOnly });
                }
                rows
            }
            Some(LoadedEntity::Note(n)) => {
                let tags = n.refs.tags.iter()
                    .map(|s| format!("#{}", s))
                    .collect::<Vec<_>>()
                    .join(" ");
                let people = n.refs.people.iter()
                    .map(|s| format!("@{}", s))
                    .collect::<Vec<_>>()
                    .join(" ");
                let mut rows = vec![
                    FieldRow { field: DetailField::Title,    label: "Title",    value: n.title.clone(),                    kind: FieldKind::Text     },
                    FieldRow { field: DetailField::Tags,     label: "Tags",     value: tags,                               kind: FieldKind::Text     },
                    FieldRow { field: DetailField::Modified, label: "Modified", value: format_utc_date(&n.updated_at),     kind: FieldKind::ReadOnly },
                    FieldRow { field: DetailField::Created,  label: "Created",  value: format_utc_date(&n.created_at),     kind: FieldKind::ReadOnly },
                ];
                if !people.is_empty() {
                    rows.push(FieldRow { field: DetailField::People, label: "People", value: people, kind: FieldKind::ReadOnly });
                }
                if !n.created_dir.is_empty() {
                    rows.push(FieldRow { field: DetailField::Dir, label: "Dir", value: n.created_dir.clone(), kind: FieldKind::ReadOnly });
                }
                rows
            }
            Some(LoadedEntity::Agenda(a)) => {
                vec![
                    FieldRow { field: DetailField::Title,  label: "Title",  value: a.title.clone(),           kind: FieldKind::Text     },
                    FieldRow { field: DetailField::Date,   label: "Date",   value: format_date(&a.date),   kind: FieldKind::DatePick },
                    FieldRow { field: DetailField::Person, label: "Person", value: a.person_slug.clone(),      kind: FieldKind::ReadOnly },
                ]
            }
            None => vec![],
        }
    }

    fn detail_entity_id(&self) -> Option<&str> {
        match &self.detail_entity {
            Some(LoadedEntity::Task(t)) => Some(&t.id),
            Some(LoadedEntity::Note(n)) => Some(&n.id),
            Some(LoadedEntity::Agenda(a)) => Some(&a.id),
            None => None,
        }
    }

    fn detail_is_private(&self) -> bool {
        match &self.detail_entity {
            Some(LoadedEntity::Task(t)) => t.private,
            Some(LoadedEntity::Note(n)) => n.private || n.title.contains("[p]"),
            _ => false,
        }
    }

    fn detail_body(&self) -> Option<String> {
        match &self.detail_entity {
            Some(LoadedEntity::Task(t)) => {
                if t.description.is_empty() { None } else { Some(t.description.clone()) }
            }
            Some(LoadedEntity::Note(n)) => {
                if n.body.is_empty() { None } else { Some(n.body.clone()) }
            }
            Some(LoadedEntity::Agenda(a)) => {
                if a.body.is_empty() { None } else { Some(a.body.clone()) }
            }
            _ => None,
        }
    }

    /// Save the current field edit back to the store and refresh.
    fn save_detail_field(&mut self) -> Option<AppMessage> {
        let val = self.detail_input.trim().to_string();
        let fields = self.detail_fields();
        let Some(row) = fields.get(self.detail_field_cursor) else { return None };
        let field = row.field;

        let result: Option<anyhow::Result<()>> = match &mut self.detail_entity {
            Some(LoadedEntity::Task(t)) => {
                match field {
                    DetailField::Title => {
                        t.title = val.clone();
                        t.updated_at = chrono::Utc::now();
                        Some(self.store.save_task(t))
                    }
                    DetailField::Tags => {
                        t.refs.tags = val.split_whitespace()
                            .map(|s| s.trim_start_matches('#').to_lowercase())
                            .filter(|s| !s.is_empty())
                            .collect();
                        t.updated_at = chrono::Utc::now();
                        Some(self.store.save_task(t))
                    }
                    _ => None,
                }
            }
            Some(LoadedEntity::Note(n)) => {
                match field {
                    DetailField::Title => {
                        n.title = val.clone();
                        n.updated_at = chrono::Utc::now();
                        Some(self.store.save_note(n))
                    }
                    DetailField::Tags => {
                        n.refs.tags = val.split_whitespace()
                            .map(|s| s.trim_start_matches('#').to_lowercase())
                            .filter(|s| !s.is_empty())
                            .collect();
                        n.updated_at = chrono::Utc::now();
                        Some(self.store.save_note(n))
                    }
                    _ => None,
                }
            }
            Some(LoadedEntity::Agenda(a)) => {
                if field == DetailField::Title {
                    a.title = val.clone();
                    a.updated_at = chrono::Utc::now();
                    Some(self.store.save_agenda(a))
                } else {
                    None
                }
            }
            None => None,
        };

        self.detail_mode = DetailMode::Normal;
        self.detail_input.clear();
        self.detail_input_cursor = 0;

        if let Some(Err(e)) = result {
            return Some(AppMessage::Error(format!("Save failed: {}", e)));
        }
        self.load_detail_item();
        self.load_day_items();
        Some(AppMessage::Reload)
    }

    /// Apply selection popup result.
    fn apply_detail_selection(&mut self) -> Option<AppMessage> {
        let result: Option<anyhow::Result<()>> = match (&mut self.detail_entity, self.detail_select_field) {
            (Some(LoadedEntity::Task(t)), DetailField::Status) => {
                let label = self.detail_select_options.get(self.detail_select_cursor).copied().unwrap_or("Backlog");
                if let Some(s) = TaskStatus::from_str_loose(label) {
                    t.status = s;
                    t.updated_at = chrono::Utc::now();
                    Some(self.store.save_task(t))
                } else { None }
            }
            (Some(LoadedEntity::Task(t)), DetailField::Priority) => {
                let idx = self.detail_select_cursor;
                t.priority = PRIORITY_OPTIONS.get(idx).copied().unwrap_or(Priority::None);
                t.updated_at = chrono::Utc::now();
                Some(self.store.save_task(t))
            }
            _ => None,
        };

        self.detail_mode = DetailMode::Normal;

        if let Some(Err(e)) = result {
            return Some(AppMessage::Error(format!("Save failed: {}", e)));
        }
        self.load_detail_item();
        self.load_day_items();
        Some(AppMessage::Reload)
    }

    // ── Drawing ───────────────────────────────────────────────────

    fn draw_month(&self, frame: &mut Frame, area: Rect, theme: &Theme, focused: bool) {
        let today = Local::now().date_naive();
        let month_label = month_name(self.month);
        let title = format!(" {} {} ", month_label, self.year);
        let title_style = if focused { theme.title } else { theme.subtitle };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // title + blank
                Constraint::Length(1), // weekday headers
                Constraint::Min(6),    // grid rows
            ])
            .split(area);

        let title_line = Line::from(Span::styled(title, title_style));
        frame.render_widget(Paragraph::new(title_line), chunks[0]);

        let headers = weekday_headers();
        let header_spans: Vec<Span> = headers
            .iter()
            .map(|h| Span::styled(format!(" {:>2}  ", h), theme.column_header))
            .collect();
        frame.render_widget(Paragraph::new(Line::from(header_spans)), chunks[1]);

        let grid_area = chunks[2];
        let available_rows = (grid_area.height as usize).min(6);
        let mut lines: Vec<Line> = Vec::new();

        for r in 0..available_rows {
            let mut spans: Vec<Span> = Vec::new();
            for c in 0..7 {
                let day = self.grid[r][c];
                if day == 0 {
                    spans.push(Span::raw("     "));
                    continue;
                }
                let is_cursor = r == self.row && c == self.col;
                let is_today = self.year == today.year()
                    && self.month == today.month()
                    && day == today.day() as u8;
                let has_events = self.event_counts.contains_key(&day);

                let dot = if has_events { "." } else { " " };
                let cell_text = format!(" {:>2}{} ", day, dot);

                let style = if is_cursor {
                    theme.selected
                } else if is_today {
                    theme.accent.add_modifier(Modifier::BOLD)
                } else if has_events {
                    theme.warning
                } else {
                    theme.dim
                };
                spans.push(Span::styled(cell_text, style));
            }
            lines.push(Line::from(spans));
        }
        frame.render_widget(Paragraph::new(lines), grid_area);
    }

    fn draw_separator(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let mut lines: Vec<Line> = Vec::new();
        for _ in 0..area.height {
            lines.push(Line::from(Span::styled("\u{2502}", theme.border)));
        }
        frame.render_widget(Paragraph::new(lines), area);
    }

    fn draw_day_panel(&self, frame: &mut Frame, area: Rect, theme: &Theme, focused: bool) {
        let date_label = format!(" {} {} ", icons::CALENDAR, format_date(&self.selected_date));
        let title_style = if focused { theme.title } else { theme.subtitle };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(1),
            ])
            .split(area);

        let header = Line::from(Span::styled(date_label, title_style));
        frame.render_widget(Paragraph::new(header), chunks[0]);

        if self.day_items.is_empty() {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled("  No events on this date", theme.dim))),
                chunks[1],
            );
            return;
        }

        let max_lines = chunks[1].height as usize;
        let mut lines: Vec<Line> = Vec::new();

        for (i, item) in self.day_items.iter().enumerate().take(max_lines) {
            let is_selected = focused && i == self.day_cursor;

            let title_text = if item.private && !self.revealed.contains(&item.id) {
                mask_private(&item.title, 8)
            } else {
                let max_w = area.width.saturating_sub(10) as usize;
                truncate(&item.title, max_w)
            };

            let mut spans = Vec::new();
            spans.push(Span::styled(format!("  {} ", item.icon()), theme.dim));
            if let Some(ref t) = item.time {
                spans.push(Span::styled(format!("{} ", t), theme.date));
            }
            let title_style = if item.private && !self.revealed.contains(&item.id) {
                theme.private
            } else if item.done {
                theme.status_done
            } else {
                theme.title.remove_modifier(Modifier::BOLD)
            };
            spans.push(Span::styled(title_text, title_style));

            let mut line = Line::from(spans);
            if is_selected {
                line = line.style(theme.row_gray);
            }
            lines.push(line);
        }

        frame.render_widget(Paragraph::new(lines), chunks[1]);
    }

    fn draw_detail_pane(&self, frame: &mut Frame, area: Rect, theme: &Theme, focused: bool) {
        let title_style = if focused { theme.title } else { theme.subtitle };
        let title_line = Line::from(Span::styled(" Details ", title_style));
        frame.render_widget(Paragraph::new(title_line), Rect { height: 1, ..area });

        if area.height < 2 {
            return;
        }
        let content_area = Rect {
            y: area.y + 1,
            height: area.height - 1,
            ..area
        };

        if self.detail_entity.is_none() {
            frame.render_widget(
                Paragraph::new(Span::styled("  —", theme.dim)),
                content_area,
            );
            return;
        }

        let fields = self.detail_fields();
        let n_fields = fields.len();
        let is_private = self.detail_is_private();
        let is_revealed = self.detail_entity.as_ref()
            .and_then(|_| self.day_items.get(self.day_cursor))
            .map(|item| self.revealed.contains(&item.id))
            .unwrap_or(false);

        // Reserve space: fields + 1 separator + body lines
        let fields_h = n_fields as u16;
        let sep_h = 1u16;
        let body_h = content_area.height.saturating_sub(fields_h + sep_h);

        let mut rows: Vec<Line> = Vec::new();

        for (i, row) in fields.iter().enumerate() {
            let is_active = focused && i == self.detail_field_cursor;
            let is_editing = is_active && self.detail_mode == DetailMode::EditingText;

            let value_str: String = if is_editing {
                let before = &self.detail_input[..self.detail_input_cursor];
                let after = &self.detail_input[self.detail_input_cursor..];
                format!("{}|{}", before, after)
            } else if is_private && !is_revealed && row.field == DetailField::Title {
                mask_private(&row.value, area.width as usize)
            } else {
                row.value.clone()
            };

            let key_style = if is_active { theme.subtitle } else { theme.dim };
            let value_style = if is_editing {
                theme.column_focus
            } else if is_private && !is_revealed && row.field == DetailField::Title {
                theme.private
            } else {
                match row.field {
                    DetailField::Status => TaskStatus::from_str_loose(&row.value)
                        .map(|s| theme.status_fg(&s))
                        .unwrap_or_else(|| theme.dim),
                    DetailField::Priority => match row.value.to_lowercase().as_str() {
                        "high"   => theme.priority_high,
                        "medium" => theme.priority_medium,
                        "low"    => theme.priority_low,
                        _        => theme.dim,
                    },
                    DetailField::Tags => theme.topic,
                    DetailField::People => theme.person,
                    DetailField::Dir => theme.dim,
                    _ => match row.kind {
                        FieldKind::ReadOnly => theme.dim,
                        FieldKind::Select | FieldKind::DatePick if is_active => theme.accent,
                        _ => theme.title.remove_modifier(Modifier::BOLD),
                    },
                }
            };

            let w = area.width.saturating_sub(DETAIL_PREFIX_W as u16) as usize;
            let truncated = truncate(&value_str, w);

            let glyph = detail_field_glyph(row.field);
            let mut line = Line::from(vec![
                Span::raw("  "),
                Span::styled(glyph, theme.dim),
                Span::raw("  "),
                Span::styled(format!("{:<KEY_W$}", row.label), key_style),
                Span::styled(truncated, value_style),
            ]);
            if is_active && !is_editing {
                line = line.style(theme.row_gray);
            }
            rows.push(line);
        }

        // Render fields
        let fields_area = Rect {
            height: fields_h.min(content_area.height),
            ..content_area
        };
        frame.render_widget(Paragraph::new(rows), fields_area);

        // Separator + body
        let below_fields = content_area.y + fields_h.min(content_area.height);
        let remaining = content_area.y + content_area.height;
        if below_fields >= remaining {
            return;
        }

        // Separator line
        let sep_area = Rect {
            y: below_fields,
            height: 1,
            ..content_area
        };
        let sep = "─".repeat(sep_area.width as usize);
        frame.render_widget(Paragraph::new(Span::styled(sep, theme.border)), sep_area);

        if below_fields + 1 >= remaining {
            return;
        }
        let body_area = Rect {
            y: below_fields + 1,
            height: (remaining - below_fields - 1).min(body_h.max(1)),
            ..content_area
        };

        // Body content
        if is_private && !is_revealed {
            frame.render_widget(
                Paragraph::new(Span::styled("  [contents hidden]", theme.private)),
                body_area,
            );
            return;
        }

        match self.detail_body() {
            None => {
                frame.render_widget(
                    Paragraph::new(Span::styled("  —", theme.dim)),
                    body_area,
                );
            }
            Some(body) => {
                let w = body_area.width as usize;
                let mut lines: Vec<Line> = Vec::new();
                for raw_line in body.lines().take(body_area.height as usize) {
                    let txt = truncate(raw_line, w);
                    lines.push(Line::from(Span::styled(format!("  {}", txt), theme.dim)));
                }
                frame.render_widget(Paragraph::new(lines), body_area);
            }
        }

        // Overlay selection popup if active
        if focused && self.detail_mode == DetailMode::Selecting {
            self.draw_detail_select_popup(frame, area, theme);
        }
    }

    fn draw_detail_select_popup(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let opts = self.detail_select_options;
        let n = opts.len() as u16;
        let popup_w = 20u16.min(area.width);
        let popup_h = (n + 2).min(area.height);

        let x = area.x + area.width.saturating_sub(popup_w) / 2;
        let y = area.y + area.height.saturating_sub(popup_h) / 3;
        let popup_rect = Rect::new(x, y, popup_w, popup_h);

        frame.render_widget(Clear, popup_rect);

        let title = match self.detail_select_field {
            DetailField::Status   => " Status ",
            DetailField::Priority => " Priority ",
            _ => " Select ",
        };
        let block = Block::default()
            .title(title)
            .title_style(theme.title)
            .borders(Borders::ALL)
            .border_style(theme.border)
            .style(theme.popup_bg);

        let inner = block.inner(popup_rect);
        frame.render_widget(block, popup_rect);

        let mut lines: Vec<Line> = Vec::new();
        for (i, opt) in opts.iter().enumerate() {
            let style = if i == self.detail_select_cursor { theme.selected } else { theme.dim };
            lines.push(Line::from(Span::styled(format!("  {}", opt), style)));
        }
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

// ── Detail pane helpers ───────────────────────────────────────────

/// Nerd Font glyph for each detail field, matching the nvim overlay header icons.
fn detail_field_glyph(field: DetailField) -> &'static str {
    match field {
        DetailField::Title    => "\u{f0219}", // 󰈙  nf-md-file_document
        DetailField::Status   => "\u{f0132}", // 󰄲  nf-md-checkbox_marked_outline
        DetailField::Priority => "\u{f04c5}", // 󰓅  nf-md-signal
        DetailField::Due      => "\u{f00f0}", // 󰃰  nf-md-calendar_clock
        DetailField::Tags     => "\u{f04f9}", // 󰓹  nf-md-tag_outline
        DetailField::Created  => "\u{f00f3}", // 󰃳  nf-md-calendar_today
        DetailField::Modified => "\u{f08a7}", // 󰢧  nf-md-calendar_edit
        DetailField::People   => "\u{f0004}", // 󰀄  nf-md-account
        DetailField::Dir      => "\u{f024b}", // 󰉋  nf-md-folder
        DetailField::Date     => "\u{f00ed}", // 󰃭  nf-md-calendar
        DetailField::Person   => "\u{f0004}", // 󰀄  nf-md-account
    }
}

impl View for CalendarView {
    fn handle_event(&mut self, event: &Event) -> Option<AppMessage> {
        let Event::Key(KeyEvent { code, .. }) = event else {
            return None;
        };

        if self.creating_task {
            return self.handle_creating_task_key(*code);
        }

        if self.confirm_delete.is_some() {
            return self.handle_confirm_delete_key(*code);
        }

        match self.focus {
            CalendarFocus::Month   => self.handle_month_key(*code),
            CalendarFocus::Day     => self.handle_day_panel_key(*code),
            CalendarFocus::Details => self.handle_details_key(*code),
        }
    }

    fn handle_message(&mut self, msg: &AppMessage) {
        match msg {
            AppMessage::Reload => {
                self.refresh_event_counts();
                self.load_day_items();
                self.load_detail_item();
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
        let (main_area, confirm_area) = if self.confirm_delete.is_some() {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(area);
            (chunks[0], Some(chunks[1]))
        } else {
            (area, None)
        };

        // Month grid: fixed width (7 cols × 5 chars = 35, +2 padding)
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(37), // month grid
                Constraint::Length(1),  // separator
                Constraint::Fill(2),    // day items (40% of remaining)
                Constraint::Length(1),  // separator
                Constraint::Fill(3),    // details (60% of remaining)
            ])
            .split(main_area);

        self.draw_month(frame, chunks[0], theme, self.focus == CalendarFocus::Month);
        self.draw_separator(frame, chunks[1], theme);
        self.draw_day_panel(frame, chunks[2], theme, self.focus == CalendarFocus::Day);
        self.draw_separator(frame, chunks[3], theme);
        self.draw_detail_pane(frame, chunks[4], theme, self.focus == CalendarFocus::Details);

        if let Some(confirm_area) = confirm_area {
            self.draw_confirm_bar(frame, confirm_area, theme);
        }

        if self.creating_task {
            self.draw_creating_task_prompt(frame, main_area, theme);
        }
    }

    fn captures_input(&self) -> bool {
        self.confirm_delete.is_some()
            || self.detail_mode != DetailMode::Normal
            || self.creating_task
    }
}

impl CalendarView {
    fn handle_month_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        match code {
            KeyCode::Char('h') | KeyCode::Left => {
                self.move_left();
                self.sync_day_from_cursor();
                None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_down();
                self.sync_day_from_cursor();
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_up();
                self.sync_day_from_cursor();
                None
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.move_right();
                self.sync_day_from_cursor();
                None
            }
            KeyCode::Char('[') => {
                self.go_prev_month();
                self.sync_day_from_cursor();
                None
            }
            KeyCode::Char(']') => {
                self.go_next_month();
                self.sync_day_from_cursor();
                None
            }
            KeyCode::Char('t') => {
                self.go_today();
                self.sync_day_from_cursor();
                None
            }
            KeyCode::Tab | KeyCode::Enter => {
                self.focus = CalendarFocus::Day;
                None
            }
            KeyCode::BackTab => {
                self.focus = CalendarFocus::Details;
                None
            }
            KeyCode::Char('n') => {
                let day = self.current_day();
                if day > 0 {
                    if let Some(date) = NaiveDate::from_ymd_opt(self.year, self.month, day as u32) {
                        self.selected_date = date;
                        self.creating_task = true;
                        self.new_task_buf.clear();
                        self.new_task_cursor = 0;
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn handle_creating_task_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        match code {
            KeyCode::Esc => {
                self.creating_task = false;
                self.new_task_buf.clear();
                self.new_task_cursor = 0;
                None
            }
            KeyCode::Enter => {
                self.creating_task = false;
                let title = if self.new_task_buf.trim().is_empty() {
                    "New task".to_string()
                } else {
                    self.new_task_buf.trim().to_string()
                };
                self.new_task_buf.clear();
                self.new_task_cursor = 0;
                let mut task = Task::default();
                task.due_date = Some(self.selected_date);
                task.title = title;
                if let Err(e) = self.store.save_task(&task) {
                    return Some(AppMessage::Error(format!("Failed to create task: {}", e)));
                }
                let id = task.id.clone();
                self.load_day_items();
                self.refresh_event_counts();
                Some(AppMessage::OpenTaskEditor(id))
            }
            KeyCode::Backspace => {
                if self.new_task_cursor > 0 {
                    let mut prev = self.new_task_cursor - 1;
                    while prev > 0 && !self.new_task_buf.is_char_boundary(prev) {
                        prev -= 1;
                    }
                    self.new_task_buf.drain(prev..self.new_task_cursor);
                    self.new_task_cursor = prev;
                }
                None
            }
            KeyCode::Left => {
                if self.new_task_cursor > 0 {
                    let mut prev = self.new_task_cursor - 1;
                    while prev > 0 && !self.new_task_buf.is_char_boundary(prev) {
                        prev -= 1;
                    }
                    self.new_task_cursor = prev;
                }
                None
            }
            KeyCode::Right => {
                if self.new_task_cursor < self.new_task_buf.len() {
                    let mut next = self.new_task_cursor + 1;
                    while next < self.new_task_buf.len() && !self.new_task_buf.is_char_boundary(next) {
                        next += 1;
                    }
                    self.new_task_cursor = next;
                }
                None
            }
            KeyCode::Home => {
                self.new_task_cursor = 0;
                None
            }
            KeyCode::End => {
                self.new_task_cursor = self.new_task_buf.len();
                None
            }
            KeyCode::Char(c) => {
                self.new_task_buf.insert(self.new_task_cursor, c);
                self.new_task_cursor += c.len_utf8();
                None
            }
            _ => None,
        }
    }

    fn handle_day_panel_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        match code {
            KeyCode::Esc | KeyCode::BackTab => {
                self.focus = CalendarFocus::Month;
                None
            }
            KeyCode::Tab => {
                self.focus = CalendarFocus::Details;
                None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.day_items.is_empty() && self.day_cursor + 1 < self.day_items.len() {
                    self.day_cursor += 1;
                    self.load_detail_item();
                }
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.day_cursor > 0 {
                    self.day_cursor -= 1;
                    self.load_detail_item();
                }
                None
            }
            KeyCode::Enter => {
                if let Some(item) = self.day_items.get(self.day_cursor) {
                    let item_id = item.id.clone();
                    let item_kind = item.kind.clone();
                    let item_private = item.private;
                    if item_private {
                        if self.revealed.contains(&item_id) {
                            self.revealed.remove(&item_id);
                        } else {
                            self.revealed.insert(item_id);
                        }
                        return None;
                    }
                    match item_kind {
                        EntityKind::Task => return Some(AppMessage::OpenTaskEditor(item_id)),
                        EntityKind::Note => return Some(AppMessage::OpenNoteEditor(item_id)),
                        EntityKind::Agenda => {
                            return Some(AppMessage::OpenInlineEditor {
                                kind: EntityKind::Agenda,
                                id: item_id,
                            });
                        }
                        _ => {}
                    }
                }
                None
            }
            KeyCode::Char('e') => {
                if let Some(item) = self.day_items.get(self.day_cursor) {
                    return Some(AppMessage::EditEntity {
                        kind: item.kind.clone(),
                        id: item.id.clone(),
                    });
                }
                None
            }
            KeyCode::Char('n') => {
                self.creating_task = true;
                self.new_task_buf.clear();
                self.new_task_cursor = 0;
                None
            }
            KeyCode::Char('d') => {
                if self.day_cursor < self.day_items.len() {
                    self.confirm_delete = Some(self.day_cursor);
                }
                None
            }
            _ => None,
        }
    }

    fn handle_details_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        // Selection popup is active — route to it
        if self.detail_mode == DetailMode::Selecting {
            return self.handle_detail_select_key(code);
        }
        // Inline text editing
        if self.detail_mode == DetailMode::EditingText {
            return self.handle_detail_edit_key(code);
        }

        // Normal navigation
        let fields = self.detail_fields();
        let n = fields.len();

        match code {
            KeyCode::Esc | KeyCode::BackTab => {
                self.focus = CalendarFocus::Day;
                None
            }
            KeyCode::Tab => {
                self.focus = CalendarFocus::Month;
                None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if n > 0 && self.detail_field_cursor + 1 < n {
                    self.detail_field_cursor += 1;
                }
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.detail_field_cursor > 0 {
                    self.detail_field_cursor -= 1;
                }
                None
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(row) = fields.get(self.detail_field_cursor) {
                    match row.kind {
                        FieldKind::Text => {
                            self.detail_input = row.value.clone();
                            self.detail_input_cursor = self.detail_input.len();
                            self.detail_mode = DetailMode::EditingText;
                        }
                        FieldKind::Select => {
                            let (options, field) = match row.field {
                                DetailField::Status   => (STATUS_OPTIONS,  DetailField::Status),
                                DetailField::Priority => (PRIORITY_LABELS, DetailField::Priority),
                                _ => return None,
                            };
                            self.detail_select_options = options;
                            self.detail_select_field = field;
                            // Pre-position cursor on current value
                            self.detail_select_cursor = options.iter()
                                .position(|o| o.to_lowercase() == row.value.to_lowercase())
                                .unwrap_or(0);
                            self.detail_mode = DetailMode::Selecting;
                        }
                        FieldKind::DatePick => {
                            // Build context from entity kind + id
                            if let Some(id) = self.detail_entity_id().map(str::to_string) {
                                let ctx = match &self.detail_entity {
                                    Some(LoadedEntity::Task(_)) => Some(DatePickerContext::TaskDue(id)),
                                    Some(LoadedEntity::Agenda(_)) => Some(DatePickerContext::AgendaDate(id)),
                                    _ => None,
                                };
                                if let Some(context) = ctx {
                                    let current = match &self.detail_entity {
                                        Some(LoadedEntity::Task(t)) => t.due_date,
                                        Some(LoadedEntity::Agenda(a)) => Some(a.date),
                                        _ => None,
                                    };
                                    return Some(AppMessage::OpenDatePicker { date: current, context });
                                }
                            }
                        }
                        FieldKind::ReadOnly => {}
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn handle_detail_edit_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        match code {
            KeyCode::Enter => self.save_detail_field(),
            KeyCode::Esc => {
                self.detail_mode = DetailMode::Normal;
                self.detail_input.clear();
                self.detail_input_cursor = 0;
                None
            }
            KeyCode::Backspace => {
                if self.detail_input_cursor > 0 {
                    let mut prev = self.detail_input_cursor - 1;
                    while prev > 0 && !self.detail_input.is_char_boundary(prev) {
                        prev -= 1;
                    }
                    self.detail_input.drain(prev..self.detail_input_cursor);
                    self.detail_input_cursor = prev;
                }
                None
            }
            KeyCode::Left => {
                if self.detail_input_cursor > 0 {
                    self.detail_input_cursor -= 1;
                }
                None
            }
            KeyCode::Right => {
                if self.detail_input_cursor < self.detail_input.len() {
                    self.detail_input_cursor += 1;
                }
                None
            }
            KeyCode::Char(c) => {
                self.detail_input.insert(self.detail_input_cursor, c);
                self.detail_input_cursor += c.len_utf8();
                None
            }
            _ => None,
        }
    }

    fn handle_detail_select_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        let n = self.detail_select_options.len();
        match code {
            KeyCode::Esc => {
                self.detail_mode = DetailMode::Normal;
                None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.detail_select_cursor + 1 < n {
                    self.detail_select_cursor += 1;
                }
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.detail_select_cursor > 0 {
                    self.detail_select_cursor -= 1;
                }
                None
            }
            KeyCode::Enter => self.apply_detail_selection(),
            _ => None,
        }
    }

    fn handle_confirm_delete_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(idx) = self.confirm_delete.take() {
                    if let Some(item) = self.day_items.get(idx) {
                        let item_id = item.id.clone();
                        let item_kind = item.kind.clone();
                        let del_result = match item_kind {
                            EntityKind::Task   => self.store.delete_task(&item_id),
                            EntityKind::Note   => self.store.delete_note(&item_id),
                            EntityKind::Agenda => self.store.delete_agenda(&item_id),
                            _ => Ok(()),
                        };
                        if let Err(e) = del_result {
                            return Some(AppMessage::Error(format!("Failed to delete: {e}")));
                        }
                        self.load_day_items();
                        self.refresh_event_counts();
                        if self.day_cursor > 0 && self.day_cursor >= self.day_items.len() {
                            self.day_cursor = self.day_items.len().saturating_sub(1);
                        }
                        self.load_detail_item();
                        return Some(AppMessage::Reload);
                    }
                }
                None
            }
            _ => {
                self.confirm_delete = None;
                None
            }
        }
    }

    fn draw_creating_task_prompt(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        // Render a one-line prompt at the bottom of the visible area.
        let prompt_area = Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(1),
            width: area.width,
            height: 1,
        };
        let before = &self.new_task_buf[..self.new_task_cursor];
        let after = &self.new_task_buf[self.new_task_cursor..];
        let max_w = area.width.saturating_sub(20) as usize;
        let display = truncate(&format!("{}|{}", before, after), max_w);
        let line = Line::from(vec![
            Span::styled(" New task title: ", theme.dim),
            Span::styled(display, theme.column_focus),
        ]);
        frame.render_widget(Paragraph::new(line), prompt_area);
    }

    fn draw_confirm_bar(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if let Some(idx) = self.confirm_delete {
            if let Some(item) = self.day_items.get(idx) {
                let title = truncate(&item.title, 30);
                let spans = vec![
                    Span::styled(format!("Delete \"{}\"? ", title), theme.warning),
                    Span::styled("(y/n)", theme.dim),
                ];
                frame.render_widget(Paragraph::new(Line::from(spans)), area);
            }
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────

fn find_day_in_grid(grid: &[[u8; 7]; 6], day: u8) -> Option<(usize, usize)> {
    for r in 0..6 {
        for c in 0..7 {
            if grid[r][c] == day {
                return Some((r, c));
            }
        }
    }
    None
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January",   2 => "February", 3 => "March",    4 => "April",
        5 => "May",        6 => "June",     7 => "July",     8 => "August",
        9 => "September", 10 => "October", 11 => "November", 12 => "December",
        _ => "Unknown",
    }
}
