use std::collections::HashSet;
use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::message::AppMessage;
use crate::app::theme::Theme;
use crate::domain::{Agenda, EntityKind, EntityRef, Person, TaskStatus};
use crate::store::Store;
use super::{icons, mask_private, truncate, View};

// ── Focus enum ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeopleFocus {
    Sidebar,
    Metadata,
    Agendas,
    Timeline,
}

impl PeopleFocus {
    fn next(self) -> Self {
        match self {
            Self::Sidebar => Self::Agendas,
            Self::Metadata => Self::Agendas,
            Self::Agendas => Self::Timeline,
            Self::Timeline => Self::Sidebar,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::Sidebar => Self::Timeline,
            Self::Metadata => Self::Sidebar,
            Self::Agendas => Self::Sidebar,
            Self::Timeline => Self::Agendas,
        }
    }
}

// ── MetaPopupFocus ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetaPopupFocus {
    Rows,
    AddButton,
    EditButton,
}

// ── AgendaColumn / SortDirection ────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgendaColumn {
    Date,
    Title,
    Tags,
}

impl AgendaColumn {
    fn next(self) -> Self {
        match self {
            Self::Date  => Self::Title,
            Self::Title => Self::Tags,
            Self::Tags  => Self::Date,
        }
    }
    fn prev(self) -> Self {
        match self {
            Self::Date  => Self::Tags,
            Self::Title => Self::Date,
            Self::Tags  => Self::Title,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortDirection {
    Ascending,
    Descending,
}

// ── MetaField: which metadata field is highlighted ──────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct MetaField(String);

// ── TimelineItem ────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct TimelineItem {
    id: String,
    kind: EntityKind,
    title: String,
    date: String,
    private: bool,
    done: bool,
}

impl TimelineItem {
    fn icon(&self) -> &'static str {
        match self.kind {
            EntityKind::Task => icons::TASK,
            EntityKind::Note => icons::NOTE,
            EntityKind::Agenda => icons::AGENDA,
            _ => icons::TASK,
        }
    }
}

// ── PeopleView ──────────────────────────────────────────────────

pub struct PeopleView {
    store: Arc<dyn Store>,

    // Left sidebar
    people: Vec<Person>,
    left_cursor: usize,

    // Right side focus
    focus: PeopleFocus,

    // Metadata section
    editing_metadata: bool,
    show_meta_popup: bool,
    meta_popup_focus: MetaPopupFocus,
    meta_editing_mode: bool,
    meta_input: String,
    meta_cursor: usize,
    meta_field: MetaField,
    /// Ordered list of metadata keys for the selected person (for stable cursor).
    meta_keys: Vec<MetaField>,

    // Two-field editing in the popup (key + value separately)
    meta_edit_in_key: bool,
    meta_key_input: String,
    // Add mode: new row at bottom of popup
    meta_add_mode: bool,
    meta_add_in_key: bool,
    meta_add_key: String,

    // 1:1 Agendas
    agendas: Vec<Agenda>,
    agenda_cursor: usize,
    agenda_editing: bool,
    agenda_edit_col: AgendaColumn,
    agenda_input: String,
    agenda_input_cursor: usize,
    agenda_sort_column: Option<AgendaColumn>,
    agenda_sort_direction: Option<SortDirection>,

    // Timeline
    timeline: Vec<TimelineItem>,
    timeline_cursor: usize,

    // Layout
    content_width: u16,
    content_height: u16,
    revealed: HashSet<String>,

    // Visibility
    show_archived: bool,

    // Global tag filter
    tag_filter: Option<String>,

    /// Pending delete confirmation: (focus context, display title, closure data).
    confirm_delete: Option<(PeopleFocus, String)>,

    // Slug editing (new person creation prompt)
    slug_editing: bool,
    slug_input: String,
}

impl PeopleView {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self {
            store,
            people: Vec::new(),
            left_cursor: 0,
            focus: PeopleFocus::Sidebar,
            editing_metadata: false,
            show_meta_popup: false,
            meta_popup_focus: MetaPopupFocus::Rows,
            meta_editing_mode: false,
            meta_input: String::new(),
            meta_cursor: 0,
            meta_field: MetaField(String::new()),
            meta_keys: Vec::new(),
            meta_edit_in_key: false,
            meta_key_input: String::new(),
            meta_add_mode: false,
            meta_add_in_key: true,
            meta_add_key: String::new(),
            agendas: Vec::new(),
            agenda_cursor: 0,
            agenda_editing: false,
            agenda_edit_col: AgendaColumn::Date,
            agenda_input: String::new(),
            agenda_input_cursor: 0,
            agenda_sort_column: None,
            agenda_sort_direction: None,
            timeline: Vec::new(),
            timeline_cursor: 0,
            content_width: 80,
            content_height: 24,
            revealed: HashSet::new(),
            show_archived: false,
            tag_filter: None,
            confirm_delete: None,
            slug_editing: false,
            slug_input: String::new(),
        }
    }

    // ── Data loading ─────────────────────────────────────────────

    fn reload(&mut self) {
        self.people.clear();

        if let Ok(people) = self.store.list_persons() {
            for p in people {
                // Filter archived
                if p.archived && !self.show_archived {
                    continue;
                }
                // Tag filter skipped for people (no direct tags field)
                // People are always shown regardless of tag filter.
                self.people.push(p);
            }
        }

        // Sort people by frecency (descending), fall back to slug for ties
        let frecency = self.store.person_frecency_scores();
        self.people.sort_by(|a, b| {
            let sa = frecency.get(&a.slug).copied().unwrap_or(0.0);
            let sb = frecency.get(&b.slug).copied().unwrap_or(0.0);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.slug.cmp(&b.slug))
        });

        // Clamp cursor
        if !self.people.is_empty() && self.left_cursor >= self.people.len() {
            self.left_cursor = self.people.len() - 1;
        }

        self.reload_right_side();
    }

    fn create_person(&mut self) {
        let default_slug = format!("person-{}", &crate::domain::new_id()[..8]);
        self.slug_editing = true;
        self.slug_input = default_slug;
    }

    fn confirm_slug_and_create(&mut self) {
        self.slug_editing = false;

        // Normalize: lowercase, non-alphanumeric → hyphen, trim leading/trailing hyphens
        let raw = self.slug_input.trim().to_string();
        let normalized: String = raw
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_string();
        let slug = if normalized.is_empty() {
            format!("person-{}", &crate::domain::new_id()[..8])
        } else {
            normalized
        };

        self.slug_input.clear();

        let person = Person {
            slug: slug.clone(),
            created_at: chrono::Utc::now(),
            pinned: false,
            archived: false,
            metadata: Default::default(),
        };
        let _ = self.store.save_person(&person);
        // Add directly to list (reload would filter out people without refs)
        self.people.push(person);
        self.people.sort_by(|a, b| a.slug.cmp(&b.slug));
        // Select the new person
        if let Some(idx) = self.people.iter().position(|p| p.slug == slug) {
            self.left_cursor = idx;
            self.reload_right_side();
        }
        // Open metadata popup and prompt for name with pre-filled key
        self.show_meta_popup = true;
        self.meta_popup_focus = MetaPopupFocus::AddButton;
        self.meta_editing_mode = false;
        self.meta_add_mode = true;
        self.meta_add_in_key = false; // start in value field since key is pre-filled
        self.meta_add_key = "name".to_string();
        self.meta_input = String::new();
        self.editing_metadata = true;
    }

    fn reload_right_side(&mut self) {
        self.reload_meta_keys();
        self.reload_agendas();
        self.reload_timeline();
    }

    fn reload_meta_keys(&mut self) {
        self.meta_keys.clear();

        if let Some(person) = self.people.get(self.left_cursor) {
            let mut keys: Vec<String> = person.metadata.keys().cloned().collect();
            keys.sort();
            for k in keys {
                self.meta_keys.push(MetaField(k));
            }
        }

        // Clamp meta cursor
        if !self.meta_keys.is_empty() && self.meta_cursor >= self.meta_keys.len() {
            self.meta_cursor = self.meta_keys.len() - 1;
        }
        if let Some(field) = self.meta_keys.get(self.meta_cursor) {
            self.meta_field = field.clone();
        }
    }

    fn reload_agendas(&mut self) {
        self.agendas.clear();
        self.agenda_cursor = 0;
        let Some(person) = self.people.get(self.left_cursor) else { return; };
        self.agendas = self.store.list_agendas_for_person(&person.slug).unwrap_or_default();
        self.sort_agendas();
    }

    fn sort_agendas(&mut self) {
        match (self.agenda_sort_column, self.agenda_sort_direction) {
            (Some(AgendaColumn::Date), Some(SortDirection::Ascending)) => {
                self.agendas.sort_by(|a, b| a.date.cmp(&b.date));
            }
            (Some(AgendaColumn::Date), Some(SortDirection::Descending)) => {
                self.agendas.sort_by(|a, b| b.date.cmp(&a.date));
            }
            (Some(AgendaColumn::Title), Some(SortDirection::Ascending)) => {
                self.agendas.sort_by(|a, b| a.title.cmp(&b.title));
            }
            (Some(AgendaColumn::Title), Some(SortDirection::Descending)) => {
                self.agendas.sort_by(|a, b| b.title.cmp(&a.title));
            }
            (Some(AgendaColumn::Tags), Some(SortDirection::Ascending)) => {
                self.agendas.sort_by(|a, b| {
                    let ta = a.refs.tags.first().map(|s| s.as_str()).unwrap_or("");
                    let tb = b.refs.tags.first().map(|s| s.as_str()).unwrap_or("");
                    ta.cmp(tb)
                });
            }
            (Some(AgendaColumn::Tags), Some(SortDirection::Descending)) => {
                self.agendas.sort_by(|a, b| {
                    let ta = a.refs.tags.first().map(|s| s.as_str()).unwrap_or("");
                    let tb = b.refs.tags.first().map(|s| s.as_str()).unwrap_or("");
                    tb.cmp(ta)
                });
            }
            _ => {
                // Default: by date descending
                self.agendas.sort_by(|a, b| b.date.cmp(&a.date));
            }
        }
    }

    fn reload_timeline(&mut self) {
        self.timeline.clear();
        self.timeline_cursor = 0;

        let Some(person) = self.people.get(self.left_cursor) else {
            return;
        };

        let refs = self.store.get_memory(&person.slug);
        for eref in &refs {
            if matches!(eref.kind, EntityKind::Person | EntityKind::Tag) {
                continue;
            }
            if let Some(item) = self.resolve_entity_ref(eref) {
                self.timeline.push(item);
            }
        }

        // Sort reverse chronological (newest first)
        self.timeline.sort_by(|a, b| b.date.cmp(&a.date));
    }

    fn resolve_entity_ref(&self, eref: &EntityRef) -> Option<TimelineItem> {
        match eref.kind {
            EntityKind::Task => {
                let id = eref.id.clone();
                if let Ok(t) = self.store.get_task(&id) {
                    let date = t.due_date.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_else(|| {
                        let local: chrono::DateTime<chrono::Local> = t.created_at.into();
                        local.format("%Y-%m-%d").to_string()
                    });
                    return Some(TimelineItem {
                        id: t.id.clone(),
                        kind: EntityKind::Task,
                        title: t.title.clone(),
                        date,
                        private: t.private,
                        done: t.status == TaskStatus::Done,
                    });
                }
            }
            EntityKind::Note => {
                let id = eref.id.clone();
                if let Ok(n) = self.store.get_note(&id) {
                    let local: chrono::DateTime<chrono::Local> = n.created_at.into();
                    return Some(TimelineItem {
                        id: n.id.clone(),
                        kind: EntityKind::Note,
                        title: n.title.clone(),
                        date: local.format("%Y-%m-%d").to_string(),
                        private: n.private,
                        done: false,
                    });
                }
            }
            EntityKind::Agenda => {
                let id = eref.id.clone();
                if let Ok(a) = self.store.get_agenda(&id) {
                    return Some(TimelineItem {
                        id: a.id.clone(),
                        kind: EntityKind::Agenda,
                        title: a.title.clone(),
                        date: a.date.format("%Y-%m-%d").to_string(),
                        private: false,
                        done: false,
                    });
                }
            }
            _ => {}
        }
        None
    }

    // ── Sidebar width ────────────────────────────────────────────


    // ── Person display name ──────────────────────────────────────

    fn person_display(&self, person: &Person) -> String {
        person.display_name()
    }

    // ── Metadata editing ─────────────────────────────────────────

    fn start_meta_edit(&mut self) {
        let Some(person) = self.people.get(self.left_cursor) else { return; };
        let MetaField(key) = &self.meta_field;
        let value = person.metadata.get(key).cloned().unwrap_or_default();

        self.meta_key_input = key.clone();
        self.meta_input = value;
        self.meta_edit_in_key = false; // default: edit value, Tab to switch to key
        self.editing_metadata = true;
    }

    fn save_meta_edit(&mut self) {
        self.editing_metadata = false;
        let Some(person) = self.people.get_mut(self.left_cursor) else {
            self.meta_input.clear();
            self.meta_key_input.clear();
            return;
        };

        let old_key = match &self.meta_field { MetaField(k) => k.clone() };
        let new_key = self.meta_key_input.trim().to_string();
        let val = self.meta_input.trim().to_string();

        // Remove old key regardless of rename
        person.metadata.remove(&old_key);

        // Re-insert only if both key and value are non-empty
        if !new_key.is_empty() && !val.is_empty() {
            person.metadata.insert(new_key.clone(), val.clone());
        }

        let _ = self.store.save_person(person);
        if new_key == "name" && !val.is_empty() {
            self.maybe_rename_person_slug(&val);
        }

        self.meta_input.clear();
        self.meta_key_input.clear();
        self.reload_meta_keys();
    }

    /// When a name is set on a person with an auto-generated slug (person-*),
    /// rename the slug to match the name so @mentions resolve correctly.
    fn maybe_rename_person_slug(&mut self, name: &str) {
        let Some(person) = self.people.get(self.left_cursor) else { return; };
        let old_slug = person.slug.clone();

        // Only rename auto-generated slugs
        if !old_slug.starts_with("person-") {
            return;
        }

        // Derive slug from name: lowercase, replace non-alphanumeric with hyphens, trim
        let new_slug: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_string();

        if new_slug.is_empty() || new_slug == old_slug {
            return;
        }

        // Check if a person with the new slug already exists
        if let Ok(mut existing) = self.store.get_person(&new_slug) {
            // Merge: copy metadata and tags from the new person into existing
            let person = &self.people[self.left_cursor];
            for (k, v) in &person.metadata {
                existing.metadata.entry(k.clone()).or_insert_with(|| v.clone());
            }
            // Person no longer has a tags field; nothing to merge.
            let _ = self.store.save_person(&existing);
            // Delete the auto-generated person
            let _ = self.store.delete_person(&old_slug);
        } else {
            // Rename: delete old, create new with updated slug
            let person = &self.people[self.left_cursor];
            let mut renamed = person.clone();
            renamed.slug = new_slug.clone();
            let _ = self.store.delete_person(&old_slug);
            let _ = self.store.save_person(&renamed);
        }

        // Reload to pick up the change
        self.reload();
        // Re-select the person by new slug
        if let Some(idx) = self.people.iter().position(|p| p.slug == new_slug) {
            self.left_cursor = idx;
            self.reload_right_side();
        }
    }

    fn cancel_meta_edit(&mut self) {
        self.editing_metadata = false;
        self.meta_add_mode = false;
        self.meta_add_key.clear();
        self.meta_key_input.clear();
        self.meta_input.clear();
    }

    fn delete_meta_field(&mut self) {
        let Some(person) = self.people.get_mut(self.left_cursor) else {
            return;
        };

        match &self.meta_field {
            MetaField(key) => {
                person.metadata.remove(key);
            }
        }

        let _ = self.store.save_person(person);
        self.reload_meta_keys();
    }

    fn add_meta_field(&mut self) {
        self.meta_add_mode = true;
        self.meta_add_in_key = true;
        self.meta_add_key = String::new();
        self.meta_input = String::new();
        self.editing_metadata = true;
    }

    fn save_new_meta_field(&mut self) {
        self.meta_add_mode = false;
        self.editing_metadata = false;

        let key = self.meta_add_key.trim().to_string();
        let val = self.meta_input.trim().to_string();

        self.meta_add_key.clear();
        self.meta_input.clear();

        if key.is_empty() {
            self.reload_meta_keys();
            return;
        }

        let Some(person) = self.people.get_mut(self.left_cursor) else {
            self.reload_meta_keys();
            return;
        };

        person.metadata.insert(key.clone(), val.clone());
        let _ = self.store.save_person(person);
        if key == "name" && !val.is_empty() {
            self.maybe_rename_person_slug(&val);
        }

        self.reload_meta_keys();
    }

    // ── Agenda operations ─────────────────────────────────────────

    fn create_agenda(&mut self) -> Option<AppMessage> {
        let Some(person) = self.people.get(self.left_cursor) else { return None; };
        let today = chrono::Local::now().date_naive();
        let now = chrono::Utc::now();
        let agenda = Agenda {
            id: crate::domain::new_id(),
            title: format!("1:1 @{}", person.slug),
            person_slug: person.slug.clone(),
            date: today,
            created_at: now,
            updated_at: now,
            body: String::new(),
            refs: Default::default(),
        };
        let id = agenda.id.clone();
        let _ = self.store.save_agenda(&agenda);
        self.reload_agendas();
        // Open the new agenda in the inline editor
        Some(AppMessage::OpenInlineEditor { kind: EntityKind::Agenda, id })
    }

    fn delete_agenda(&mut self) {
        if let Some(agenda) = self.agendas.get(self.agenda_cursor).cloned() {
            let _ = self.store.delete_agenda(&agenda.id);
            self.reload_agendas();
            if self.agenda_cursor > 0 && self.agenda_cursor >= self.agendas.len() {
                self.agenda_cursor = self.agendas.len().saturating_sub(1);
            }
        }
    }

    fn start_agenda_edit(&mut self) {
        let Some(agenda) = self.agendas.get(self.agenda_cursor) else { return; };
        self.agenda_editing = true;
        self.agenda_input = match self.agenda_edit_col {
            AgendaColumn::Date  => agenda.date.format("%Y-%m-%d").to_string(),
            AgendaColumn::Title => agenda.title.clone(),
            AgendaColumn::Tags  => agenda.refs.tags.iter()
                .map(|t| format!("#{}", t))
                .collect::<Vec<_>>()
                .join(" "),
        };
        self.agenda_input_cursor = self.agenda_input.len();
    }

    fn save_agenda_edit(&mut self) {
        self.agenda_editing = false;
        let Some(agenda) = self.agendas.get_mut(self.agenda_cursor) else {
            self.agenda_input.clear();
            return;
        };

        match self.agenda_edit_col {
            AgendaColumn::Date => {
                let trimmed = self.agenda_input.trim();
                if let Ok(parsed_date) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
                    agenda.date = parsed_date;
                }
            }
            AgendaColumn::Title => {
                let trimmed = self.agenda_input.trim().to_string();
                if !trimmed.is_empty() {
                    agenda.title = trimmed;
                }
            }
            AgendaColumn::Tags => {
                agenda.refs.tags = self.agenda_input.split_whitespace()
                    .map(|s| s.trim_start_matches('#').to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }

        agenda.updated_at = chrono::Utc::now();
        let _ = self.store.save_agenda(agenda);
        self.agenda_input.clear();
        self.reload_agendas();
    }

    fn cancel_agenda_edit(&mut self) {
        self.agenda_editing = false;
        self.agenda_input.clear();
    }

    fn cycle_agenda_sort(&mut self) {
        match (self.agenda_sort_column, self.agenda_sort_direction) {
            (Some(col), Some(SortDirection::Ascending)) if col == self.agenda_edit_col => {
                self.agenda_sort_column = Some(col);
                self.agenda_sort_direction = Some(SortDirection::Descending);
            }
            (Some(col), Some(SortDirection::Descending)) if col == self.agenda_edit_col => {
                self.agenda_sort_column = None;
                self.agenda_sort_direction = None;
            }
            _ => {
                self.agenda_sort_column = Some(self.agenda_edit_col);
                self.agenda_sort_direction = Some(SortDirection::Ascending);
            }
        }
        self.sort_agendas();
    }
}

// ── View trait ────────────────────────────────────────────────────

impl View for PeopleView {
    fn handle_event(&mut self, event: &Event) -> Option<AppMessage> {
        let Event::Key(KeyEvent { code, .. }) = event else {
            return None;
        };

        if self.confirm_delete.is_some() {
            return self.handle_confirm_delete_key(*code);
        }

        if self.slug_editing {
            return self.handle_slug_edit_key(*code);
        }

        if self.show_meta_popup {
            if self.editing_metadata {
                return self.handle_meta_edit_key(*code);
            }
            return self.handle_meta_popup_key(*code);
        }

        if self.agenda_editing {
            return self.handle_agenda_edit_key(*code);
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
            AppMessage::NavigatePerson(slug) => {
                self.reload();
                // Find and select the person by slug
                if let Some(idx) = self.people.iter().position(|p| p.slug == *slug) {
                    self.left_cursor = idx;
                    self.focus = PeopleFocus::Sidebar;
                    self.reload_right_side();
                }
            }
            _ => {}
        }
    }

    fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if self.people.is_empty() {
            let empty = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled("No people found", theme.title)),
                Line::from(""),
                Line::from(Span::styled(
                    "Press 'n' to create a person, or use @mentions in tasks/notes/sink",
                    theme.dim,
                )),
            ])
            .alignment(ratatui::layout::Alignment::Center);
            frame.render_widget(empty, area);
            return;
        }

        // Allocate confirm bar if needed
        let (main_area, confirm_area) = if self.confirm_delete.is_some() {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(area);
            (chunks[0], Some(chunks[1]))
        } else {
            (area, None)
        };

        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(20),
                Constraint::Min(1),
            ])
            .split(main_area);

        self.draw_sidebar(frame, h_chunks[0], theme);
        self.draw_right(frame, h_chunks[1], theme);

        if let Some(confirm_area) = confirm_area {
            self.draw_confirm_bar(frame, confirm_area, theme);
        }

        if self.show_meta_popup {
            self.draw_meta_popup(frame, main_area, theme);
        }

        if self.slug_editing {
            self.draw_slug_edit_popup(frame, main_area, theme);
        }
    }

    fn captures_input(&self) -> bool {
        self.slug_editing || self.show_meta_popup || self.editing_metadata || self.agenda_editing || self.confirm_delete.is_some()
    }
}

impl PeopleView {
    fn draw_confirm_bar(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if let Some((_focus, ref title)) = self.confirm_delete {
            let title = truncate(title, 30);
            let spans = vec![
                Span::styled(format!("Delete \"{}\"? ", title), theme.warning),
                Span::styled("(y/n)", theme.dim),
            ];
            let line = Line::from(spans);
            frame.render_widget(Paragraph::new(line), area);
        }
    }

    fn handle_normal_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        match code {
            KeyCode::Tab => {
                self.focus = self.focus.next();
                None
            }
            KeyCode::BackTab => {
                self.focus = self.focus.prev();
                None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                match self.focus {
                    PeopleFocus::Sidebar => {
                        if !self.people.is_empty()
                            && self.left_cursor + 1 < self.people.len()
                        {
                            self.left_cursor += 1;
                            self.reload_right_side();
                        }
                    }
                    PeopleFocus::Metadata => {
                        if !self.meta_keys.is_empty()
                            && self.meta_cursor + 1 < self.meta_keys.len()
                        {
                            self.meta_cursor += 1;
                            self.meta_field = self.meta_keys[self.meta_cursor].clone();
                        }
                    }
                    PeopleFocus::Agendas => {
                        if !self.agendas.is_empty()
                            && self.agenda_cursor + 1 < self.agendas.len()
                        {
                            self.agenda_cursor += 1;
                        }
                    }
                    PeopleFocus::Timeline => {
                        if !self.timeline.is_empty()
                            && self.timeline_cursor + 1 < self.timeline.len()
                        {
                            self.timeline_cursor += 1;
                        }
                    }
                }
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                match self.focus {
                    PeopleFocus::Sidebar => {
                        if self.left_cursor > 0 {
                            self.left_cursor -= 1;
                            self.reload_right_side();
                        }
                    }
                    PeopleFocus::Metadata => {
                        if self.meta_cursor > 0 {
                            self.meta_cursor -= 1;
                            self.meta_field = self.meta_keys[self.meta_cursor].clone();
                        }
                    }
                    PeopleFocus::Agendas => {
                        if self.agenda_cursor > 0 {
                            self.agenda_cursor -= 1;
                        }
                    }
                    PeopleFocus::Timeline => {
                        if self.timeline_cursor > 0 {
                            self.timeline_cursor -= 1;
                        }
                    }
                }
                None
            }
            KeyCode::Char('g') => {
                match self.focus {
                    PeopleFocus::Sidebar => {
                        self.left_cursor = 0;
                        self.reload_right_side();
                    }
                    PeopleFocus::Metadata => {
                        self.meta_cursor = 0;
                        if let Some(f) = self.meta_keys.first() {
                            self.meta_field = f.clone();
                        }
                    }
                    PeopleFocus::Agendas => {
                        self.agenda_cursor = 0;
                    }
                    PeopleFocus::Timeline => {
                        self.timeline_cursor = 0;
                    }
                }
                None
            }
            KeyCode::Char('G') => {
                match self.focus {
                    PeopleFocus::Sidebar => {
                        if !self.people.is_empty() {
                            self.left_cursor = self.people.len() - 1;
                            self.reload_right_side();
                        }
                    }
                    PeopleFocus::Metadata => {
                        if !self.meta_keys.is_empty() {
                            self.meta_cursor = self.meta_keys.len() - 1;
                            self.meta_field = self.meta_keys[self.meta_cursor].clone();
                        }
                    }
                    PeopleFocus::Agendas => {
                        if !self.agendas.is_empty() {
                            self.agenda_cursor = self.agendas.len() - 1;
                        }
                    }
                    PeopleFocus::Timeline => {
                        if !self.timeline.is_empty() {
                            self.timeline_cursor = self.timeline.len() - 1;
                        }
                    }
                }
                None
            }
            KeyCode::Char('e') => {
                match self.focus {
                    PeopleFocus::Sidebar => {
                        // Switch to metadata focus; if fields exist, edit first one; otherwise add new
                        self.focus = PeopleFocus::Metadata;
                        if self.meta_keys.is_empty() {
                            self.add_meta_field();
                        } else {
                            self.meta_cursor = 0;
                            self.meta_field = self.meta_keys[0].clone();
                            self.start_meta_edit();
                        }
                        None
                    }
                    PeopleFocus::Metadata => {
                        if self.meta_keys.is_empty() {
                            self.add_meta_field();
                        } else {
                            self.start_meta_edit();
                        }
                        None
                    }
                    PeopleFocus::Agendas => {
                        if !self.agendas.is_empty() {
                            if self.agenda_edit_col == AgendaColumn::Date {
                                let agenda = &self.agendas[self.agenda_cursor];
                                let current = Some(agenda.date);
                                return Some(AppMessage::OpenDatePicker {
                                    date: current,
                                    context: crate::app::message::DatePickerContext::AgendaDate(
                                        agenda.id.clone(),
                                    ),
                                });
                            } else {
                                self.start_agenda_edit();
                            }
                        }
                        None
                    }
                    PeopleFocus::Timeline => {
                        if let Some(item) = self.timeline.get(self.timeline_cursor) {
                            return Some(AppMessage::EditEntity {
                                kind: item.kind.clone(),
                                id: item.id.clone(),
                            });
                        }
                        None
                    }
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if self.focus == PeopleFocus::Agendas {
                    self.agenda_edit_col = self.agenda_edit_col.prev();
                }
                None
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if self.focus == PeopleFocus::Agendas {
                    self.agenda_edit_col = self.agenda_edit_col.next();
                }
                None
            }
            KeyCode::Char('S') => {
                if self.focus == PeopleFocus::Agendas {
                    self.cycle_agenda_sort();
                }
                None
            }
            KeyCode::Char('n') => {
                match self.focus {
                    PeopleFocus::Sidebar => {
                        self.create_person();
                        None
                    }
                    PeopleFocus::Metadata => {
                        self.add_meta_field();
                        None
                    }
                    PeopleFocus::Agendas => {
                        self.create_agenda()
                    }
                    _ => None,
                }
            }
            KeyCode::Char('d') => {
                match self.focus {
                    PeopleFocus::Metadata => {
                        // Metadata fields: ask confirmation
                        if let Some(MetaField(key)) = self.meta_keys.get(self.meta_cursor) {
                            if !key.is_empty() {
                                self.confirm_delete = Some((PeopleFocus::Metadata, key.clone()));
                            }
                        }
                        None
                    }
                    PeopleFocus::Agendas => {
                        if let Some(agenda) = self.agendas.get(self.agenda_cursor) {
                            self.confirm_delete = Some((PeopleFocus::Agendas, agenda.title.clone()));
                        }
                        None
                    }
                    PeopleFocus::Timeline => {
                        if let Some(item) = self.timeline.get(self.timeline_cursor) {
                            self.confirm_delete = Some((PeopleFocus::Timeline, item.title.clone()));
                        }
                        None
                    }
                    _ => None,
                }
            }
            KeyCode::Enter => {
                match self.focus {
                    PeopleFocus::Sidebar => {
                        self.show_meta_popup = true;
                        self.meta_popup_focus = if self.meta_keys.is_empty() {
                            MetaPopupFocus::AddButton
                        } else {
                            MetaPopupFocus::Rows
                        };
                        self.meta_editing_mode = false;
                        None
                    }
                    PeopleFocus::Metadata => {
                        self.start_meta_edit();
                        None
                    }
                    PeopleFocus::Agendas => {
                        if let Some(agenda) = self.agendas.get(self.agenda_cursor) {
                            if self.agenda_edit_col == AgendaColumn::Date {
                                let current = Some(agenda.date);
                                return Some(AppMessage::OpenDatePicker {
                                    date: current,
                                    context: crate::app::message::DatePickerContext::AgendaDate(
                                        agenda.id.clone(),
                                    ),
                                });
                            }
                            return Some(AppMessage::OpenInlineEditor {
                                kind: EntityKind::Agenda,
                                id: agenda.id.clone(),
                            });
                        }
                        None
                    }
                    PeopleFocus::Timeline => {
                        if let Some(item) = self.timeline.get(self.timeline_cursor) {
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
                                EntityKind::Task => {
                                    return Some(AppMessage::OpenTaskEditor(item_id));
                                }
                                EntityKind::Note => {
                                    return Some(AppMessage::OpenNoteEditor(item_id));
                                }
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
                }
            }
            KeyCode::Char('a') => {
                if self.focus == PeopleFocus::Sidebar {
                    if let Some(person) = self.people.get(self.left_cursor).cloned() {
                        let mut updated = person;
                        updated.archived = !updated.archived;
                        if updated.archived {
                            updated.pinned = false; // archived implies not pinned
                        }
                        let _ = self.store.save_person(&updated);
                        self.reload();
                    }
                }
                None
            }
            KeyCode::Char('A') => {
                self.show_archived = !self.show_archived;
                self.reload();
                None
            }
            KeyCode::Char('p') => {
                if self.focus == PeopleFocus::Sidebar {
                    if let Some(person) = self.people.get(self.left_cursor).cloned() {
                        let mut updated = person;
                        updated.pinned = !updated.pinned;
                        let _ = self.store.save_person(&updated);
                        self.reload();
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn handle_meta_edit_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        match code {
            KeyCode::Enter => {
                if self.meta_add_mode {
                    if self.meta_add_in_key {
                        // Move focus from key to value field
                        self.meta_add_in_key = false;
                        return None;
                    }
                    self.save_new_meta_field();
                    return Some(AppMessage::Reload);
                }
                self.save_meta_edit();
                Some(AppMessage::Reload)
            }
            KeyCode::Tab => {
                if self.meta_add_mode {
                    self.meta_add_in_key = !self.meta_add_in_key;
                } else {
                    self.meta_edit_in_key = !self.meta_edit_in_key;
                }
                None
            }
            KeyCode::Esc => {
                self.cancel_meta_edit();
                None
            }
            KeyCode::Backspace => {
                if self.meta_add_mode {
                    if self.meta_add_in_key { self.meta_add_key.pop(); }
                    else { self.meta_input.pop(); }
                } else if self.meta_edit_in_key {
                    self.meta_key_input.pop();
                } else {
                    self.meta_input.pop();
                }
                None
            }
            KeyCode::Char(c) => {
                if self.meta_add_mode {
                    if self.meta_add_in_key { self.meta_add_key.push(c); }
                    else { self.meta_input.push(c); }
                } else if self.meta_edit_in_key {
                    self.meta_key_input.push(c);
                } else {
                    self.meta_input.push(c);
                }
                None
            }
            _ => None,
        }
    }

    fn handle_slug_edit_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        match code {
            KeyCode::Enter => {
                self.confirm_slug_and_create();
                Some(AppMessage::Reload)
            }
            KeyCode::Esc => {
                self.slug_editing = false;
                self.slug_input.clear();
                None
            }
            KeyCode::Backspace => {
                self.slug_input.pop();
                None
            }
            KeyCode::Char(c) => {
                self.slug_input.push(c);
                None
            }
            _ => None,
        }
    }

    fn draw_slug_edit_popup(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let popup_w = 48u16.min(area.width.saturating_sub(4)).max(32);
        let popup_h = 5u16;

        let popup_rect = Rect {
            x: area.x + area.width.saturating_sub(popup_w) / 2,
            y: area.y + area.height.saturating_sub(popup_h) / 2,
            width: popup_w,
            height: popup_h,
        };

        frame.render_widget(Clear, popup_rect);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border)
            .title(Span::styled(" New Person ", theme.title));

        let inner = block.inner(popup_rect);
        frame.render_widget(block, popup_rect);

        let max_w = inner.width.saturating_sub(6) as usize;
        let input_display = format!("@{}|", self.slug_input);
        let input_display = truncate(&input_display, max_w);

        let lines = vec![
            Line::from(vec![
                Span::styled("  Slug  ", theme.dim),
                Span::styled(input_display, theme.person),
            ]),
            Line::from(""),
            Line::from(Span::styled("  Enter to confirm · Esc to cancel", theme.dim)),
        ];

        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn handle_meta_popup_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        match code {
            KeyCode::Esc => {
                if self.meta_editing_mode {
                    self.meta_editing_mode = false;
                    self.meta_popup_focus = MetaPopupFocus::EditButton;
                } else {
                    self.show_meta_popup = false;
                }
                None
            }
            KeyCode::Tab => {
                self.meta_popup_focus = match self.meta_popup_focus {
                    MetaPopupFocus::Rows => MetaPopupFocus::AddButton,
                    MetaPopupFocus::AddButton => MetaPopupFocus::EditButton,
                    MetaPopupFocus::EditButton => MetaPopupFocus::Rows,
                };
                None
            }
            KeyCode::BackTab => {
                self.meta_popup_focus = match self.meta_popup_focus {
                    MetaPopupFocus::Rows => MetaPopupFocus::EditButton,
                    MetaPopupFocus::AddButton => MetaPopupFocus::Rows,
                    MetaPopupFocus::EditButton => MetaPopupFocus::AddButton,
                };
                None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.meta_popup_focus == MetaPopupFocus::Rows || self.meta_editing_mode {
                    if !self.meta_keys.is_empty() && self.meta_cursor + 1 < self.meta_keys.len() {
                        self.meta_cursor += 1;
                        self.meta_field = self.meta_keys[self.meta_cursor].clone();
                    }
                }
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.meta_popup_focus == MetaPopupFocus::Rows || self.meta_editing_mode {
                    if self.meta_cursor > 0 {
                        self.meta_cursor -= 1;
                        self.meta_field = self.meta_keys[self.meta_cursor].clone();
                    }
                }
                None
            }
            KeyCode::Enter => {
                match self.meta_popup_focus {
                    MetaPopupFocus::AddButton => {
                        self.add_meta_field();
                        None
                    }
                    MetaPopupFocus::EditButton => {
                        if !self.meta_keys.is_empty() {
                            self.meta_editing_mode = true;
                            self.meta_popup_focus = MetaPopupFocus::Rows;
                            if self.meta_cursor >= self.meta_keys.len() {
                                self.meta_cursor = 0;
                            }
                            self.meta_field = self.meta_keys[self.meta_cursor].clone();
                        } else {
                            self.add_meta_field();
                        }
                        None
                    }
                    MetaPopupFocus::Rows => {
                        if self.meta_editing_mode {
                            if self.meta_keys.is_empty() {
                                self.add_meta_field();
                            } else {
                                self.start_meta_edit();
                            }
                        }
                        None
                    }
                }
            }
            KeyCode::Char('d') => {
                if self.meta_editing_mode {
                    if let Some(MetaField(key)) = self.meta_keys.get(self.meta_cursor) {
                        if !key.is_empty() {
                            self.confirm_delete = Some((PeopleFocus::Metadata, key.clone()));
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn handle_confirm_delete_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some((focus, _title)) = self.confirm_delete.take() {
                    match focus {
                        PeopleFocus::Metadata => {
                            self.delete_meta_field();
                        }
                        PeopleFocus::Agendas => {
                            self.delete_agenda();
                        }
                        PeopleFocus::Timeline => {
                            if let Some(item) = self.timeline.get(self.timeline_cursor) {
                                let item_id = item.id.clone();
                                let item_kind = item.kind.clone();
                                match item_kind {
                                    EntityKind::Task => { let _ = self.store.delete_task(&item_id); }
                                    EntityKind::Note => { let _ = self.store.delete_note(&item_id); }
                                    EntityKind::Agenda => { let _ = self.store.delete_agenda(&item_id); }
                                    _ => {}
                                }
                                self.reload();
                            }
                        }
                        _ => {}
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

    fn handle_agenda_edit_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        match code {
            KeyCode::Enter => {
                self.save_agenda_edit();
                Some(AppMessage::Reload)
            }
            KeyCode::Esc => {
                self.cancel_agenda_edit();
                None
            }
            KeyCode::Tab => {
                self.save_agenda_edit();
                self.agenda_edit_col = self.agenda_edit_col.next();
                self.start_agenda_edit();
                None
            }
            KeyCode::BackTab => {
                self.save_agenda_edit();
                self.agenda_edit_col = self.agenda_edit_col.prev();
                self.start_agenda_edit();
                None
            }
            KeyCode::Backspace => {
                self.agenda_input.pop();
                None
            }
            KeyCode::Char(c) => {
                self.agenda_input.push(c);
                None
            }
            _ => None,
        }
    }

    // ── Drawing ──────────────────────────────────────────────────

    /// Returns (title_style, border_style) for a section based on focus.
    fn section_styles(&self, focused: bool, theme: &Theme) -> (Style, Style) {
        if focused {
            (
                theme.title,
                theme.title.remove_modifier(ratatui::style::Modifier::BOLD),
            )
        } else {
            (
                theme.dim,
                theme.border,
            )
        }
    }

    fn draw_sidebar(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let focused = self.focus == PeopleFocus::Sidebar;
        let (title_style, border_style) = self.section_styles(focused, theme);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Span::styled(" People ", title_style));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let visible_rows = inner.height as usize;
        let scroll = if self.left_cursor >= visible_rows {
            self.left_cursor - visible_rows + 1
        } else {
            0
        };

        let mut lines: Vec<Line> = Vec::new();

        for (i, person) in self.people.iter().enumerate().skip(scroll).take(visible_rows) {
            let is_selected = i == self.left_cursor;
            let display = self.person_display(person);
            let max_name_w = inner.width.saturating_sub(3) as usize;
            let display = truncate(&display, max_name_w);

            let prefix = if person.archived {
                format!("{} ", icons::ARCHIVE)
            } else if person.pinned {
                format!("{} ", icons::PIN)
            } else {
                "  ".to_string()
            };
            let prefix_style = if person.archived { theme.dim } else { theme.error };
            let name_style = if person.archived { theme.dim } else { theme.person };
            let spans = vec![
                Span::styled(prefix, prefix_style),
                Span::styled(display, name_style),
            ];

            let mut line = Line::from(spans);
            if is_selected {
                line = line.style(if focused {
                    theme.selected
                } else {
                    theme.row_gray
                });
            }
            lines.push(line);
        }

        let para = Paragraph::new(lines);
        frame.render_widget(para, inner);
    }




    fn draw_right(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        self.draw_bottom(frame, area, theme);
    }

    #[allow(dead_code)]
    fn draw_metadata(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let focused = self.focus == PeopleFocus::Metadata;
        let (title_style, border_style) = self.section_styles(focused, theme);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Span::styled(" Info ", title_style));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let mut lines: Vec<Line> = Vec::new();

        let visible_rows = inner.height as usize;
        let max_label_w = inner.width.saturating_sub(2) as usize;

        if let Some(person) = self.people.get(self.left_cursor) {
            for (i, field) in self.meta_keys.iter().enumerate().take(visible_rows) {
                let is_selected = self.focus == PeopleFocus::Metadata && i == self.meta_cursor;

                let (label, value) = match field {
                    MetaField(key) => {
                        (key.clone(), person.metadata.get(key).cloned().unwrap_or_default())
                    }
                };

                let display_text = if self.editing_metadata && is_selected {
                    format!("  {}: {}|", label, self.meta_input)
                } else if value.is_empty() {
                    format!("  {}: -", label)
                } else {
                    format!("  {}: {}", label, value)
                };

                let display_text = truncate(&display_text, max_label_w);

                let style = if self.editing_metadata && is_selected {
                    theme.column_focus
                } else if is_selected {
                    theme.column_focus
                } else {
                    theme.dim
                };

                let mut line = Line::from(Span::styled(display_text, style));
                if is_selected && !self.editing_metadata {
                    line = line.style(theme.row_gray);
                }
                lines.push(line);
            }
        }

        // If adding a new field (editing with empty key), show the input line
        if self.editing_metadata {
            let MetaField(ref key) = self.meta_field;
            if key.is_empty() && lines.len() < area.height as usize {
                let input_text = format!("  > {}|", self.meta_input);
                lines.push(Line::from(Span::styled(
                    truncate(&input_text, max_label_w),
                    theme.column_focus,
                )));
            }
        }

        // Show hint when metadata section is focused and there's room
        if self.focus == PeopleFocus::Metadata && !self.editing_metadata {
            if lines.len() < visible_rows {
                lines.push(Line::from(Span::styled(
                    " e:edit  n:add (key: value)  d:delete",
                    theme.dim,
                )));
            }
        }

        let para = Paragraph::new(lines);
        frame.render_widget(para, inner);
    }

    fn draw_meta_popup(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let Some(person) = self.people.get(self.left_cursor) else { return; };

        let num_fields = self.meta_keys.len();
        let extra_rows = if self.meta_add_mode { 1u16 } else { 0u16 };
        let content_rows = ((num_fields as u16 + extra_rows).max(1)).min(area.height.saturating_sub(6));
        // border(2) + title(1) + fields + gap(1) + buttons(1) = fields + 5
        let popup_h = (content_rows + 5).max(7).min(area.height.saturating_sub(2));
        let popup_w = 52u16.min(area.width.saturating_sub(4)).max(32);

        let popup_rect = Rect {
            x: area.x + area.width.saturating_sub(popup_w) / 2,
            y: area.y + area.height.saturating_sub(popup_h) / 2,
            width: popup_w,
            height: popup_h,
        };

        frame.render_widget(Clear, popup_rect);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border)
            .title(Span::styled(format!(" @{} ", person.slug), theme.person));

        let inner = block.inner(popup_rect);
        frame.render_widget(block, popup_rect);

        // Key and value column widths
        let max_key_w: usize = 14;
        let max_val_w = (inner.width as usize).saturating_sub(max_key_w + 4);

        let mut lines: Vec<Line> = Vec::new();

        if num_fields == 0 && !self.meta_add_mode {
            lines.push(Line::from(Span::styled("  No metadata yet", theme.dim)));
        } else {
            for (i, MetaField(key)) in self.meta_keys.iter().enumerate() {
                let value = person.metadata.get(key).cloned().unwrap_or_default();

                let is_cursor = (self.meta_popup_focus == MetaPopupFocus::Rows || self.meta_editing_mode)
                    && i == self.meta_cursor;
                let is_editing = self.editing_metadata && is_cursor;

                // Key display: show text cursor when actively editing the key field
                let key_display = if is_editing {
                    let s = format!("{}{}", self.meta_key_input,
                        if self.meta_edit_in_key { "|" } else { "" });
                    format!("{:<width$}", truncate(&s, max_key_w), width = max_key_w)
                } else {
                    format!("{:<width$}", truncate(key, max_key_w), width = max_key_w)
                };

                // Value display: show text cursor when actively editing the value field
                let val_display = if is_editing {
                    let s = format!("{}{}", self.meta_input,
                        if !self.meta_edit_in_key { "|" } else { "" });
                    truncate(&s, max_val_w)
                } else if value.is_empty() {
                    "\u{2014}".to_string()
                } else {
                    truncate(&value, max_val_w)
                };

                let key_style = if is_editing && self.meta_edit_in_key {
                    theme.column_focus
                } else if is_cursor {
                    theme.accent
                } else {
                    theme.dim
                };
                let val_style = if is_editing && !self.meta_edit_in_key {
                    theme.column_focus
                } else {
                    Style::default()
                };

                let mut line = Line::from(vec![
                    Span::raw("  "),
                    Span::styled(key_display, key_style),
                    Span::raw("  "),
                    Span::styled(val_display, val_style),
                ]);
                if is_cursor && !self.editing_metadata {
                    line = line.style(theme.row_gray);
                }
                lines.push(line);
            }

            // New-field row (shown when Add mode is active)
            if self.meta_add_mode {
                let key_text = format!("{}{}", self.meta_add_key,
                    if self.meta_add_in_key { "|" } else { "" });
                let val_text = format!("{}{}", self.meta_input,
                    if !self.meta_add_in_key { "|" } else { "" });

                let key_str = format!("{:<width$}", truncate(&key_text, max_key_w), width = max_key_w);
                let val_str = truncate(&val_text, max_val_w);

                let key_style = if self.meta_add_in_key { theme.column_focus } else { theme.dim };
                let val_style = if !self.meta_add_in_key { theme.column_focus } else { Style::default() };

                let line = Line::from(vec![
                    Span::raw("  "),
                    Span::styled(key_str, key_style),
                    Span::raw("  "),
                    Span::styled(val_str, val_style),
                ]).style(theme.row_gray);
                lines.push(line);
            }
        }

        // Render field rows (leave 1 row for the button/hint bar)
        let fields_h = inner.height.saturating_sub(1);
        frame.render_widget(
            Paragraph::new(lines),
            Rect { y: inner.y, height: fields_h, ..inner },
        );

        // Bottom bar: keyboard hints in edit mode, buttons otherwise
        let btn_y = inner.y + inner.height.saturating_sub(1);
        if self.meta_editing_mode && !self.editing_metadata {
            // Edit-mode navigation: show available keys
            let hint = Line::from(vec![
                Span::raw("  "),
                Span::styled("⏎", theme.accent),
                Span::styled(" edit  ", theme.dim),
                Span::styled("d", theme.accent),
                Span::styled(" delete  ", theme.dim),
                Span::styled("Esc", theme.accent),
                Span::styled(" exit", theme.dim),
            ]);
            frame.render_widget(
                Paragraph::new(hint),
                Rect { y: btn_y, height: 1, ..inner },
            );
        } else {
            // Normal popup: show Add / Edit buttons
            let add_focused = self.meta_popup_focus == MetaPopupFocus::AddButton || self.meta_add_mode;
            let edit_focused = self.meta_popup_focus == MetaPopupFocus::EditButton;

            // Focused button: row_gray background + accent foreground (primary color).
            // Unfocused button: row_gray background (gray, dim text).
            let focused_btn = theme.row_gray.patch(theme.accent).add_modifier(Modifier::BOLD);
            let unfocused_btn = theme.row_gray.patch(Style::default().fg(
                // Use the dim fg color for the label text
                theme.dim.fg.unwrap_or(ratatui::style::Color::Reset)
            ));

            let add_style = if add_focused { focused_btn } else { unfocused_btn };
            let edit_style = if edit_focused { focused_btn } else { unfocused_btn };

            let btn_line = Line::from(vec![
                Span::raw("  "),
                Span::styled(" Add ", add_style),
                Span::raw("    "),
                Span::styled(" Edit ", edit_style),
            ]);
            frame.render_widget(
                Paragraph::new(btn_line),
                Rect { y: btn_y, height: 1, ..inner },
            );
        }
    }

    fn draw_bottom(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(65),
                Constraint::Percentage(35),
            ])
            .split(area);

        self.draw_agendas(frame, h_chunks[0], theme);
        self.draw_timeline(frame, h_chunks[1], theme);
    }

    fn draw_agendas(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let focused = self.focus == PeopleFocus::Agendas;
        let (title_style, border_style) = self.section_styles(focused, theme);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Span::styled(" Agendas ", title_style));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let date_w: usize = 10;
        let tags_w: usize = 14;
        let title_w = (inner.width as usize).saturating_sub(date_w + tags_w + 3); // 1 leading + 2 separators

        // Sort arrows
        let date_arrow = match (self.agenda_sort_column, self.agenda_sort_direction) {
            (Some(AgendaColumn::Date), Some(SortDirection::Ascending)) => " \u{2191}",
            (Some(AgendaColumn::Date), Some(SortDirection::Descending)) => " \u{2193}",
            _ => "",
        };
        let title_arrow = match (self.agenda_sort_column, self.agenda_sort_direction) {
            (Some(AgendaColumn::Title), Some(SortDirection::Ascending)) => " \u{2191}",
            (Some(AgendaColumn::Title), Some(SortDirection::Descending)) => " \u{2193}",
            _ => "",
        };
        let tags_arrow = match (self.agenda_sort_column, self.agenda_sort_direction) {
            (Some(AgendaColumn::Tags), Some(SortDirection::Ascending)) => " \u{2191}",
            (Some(AgendaColumn::Tags), Some(SortDirection::Descending)) => " \u{2193}",
            _ => "",
        };

        // Header row
        let col_header_style = theme.column_header;
        let header = Line::from(vec![
            Span::styled(" ", col_header_style),
            Span::styled(format!("{:<width$}", format!("DATE{}", date_arrow), width = date_w), col_header_style),
            Span::styled(" ", theme.dim),
            Span::styled(format!("{:<width$}", format!("TITLE{}", title_arrow), width = title_w), col_header_style),
            Span::styled(" ", theme.dim),
            Span::styled(format!("TAGS{}", tags_arrow), col_header_style),
        ]);

        let visible_rows = inner.height.saturating_sub(1) as usize; // 1 for header
        let scroll = if self.agenda_cursor >= visible_rows {
            self.agenda_cursor - visible_rows + 1
        } else {
            0
        };

        let mut lines: Vec<Line> = vec![header];

        if self.agendas.is_empty() {
            lines.push(Line::from(Span::styled(
                " No agendas yet. Press 'n' to create one.",
                theme.dim,
            )));
        } else {
            for (i, agenda) in self.agendas.iter().enumerate().skip(scroll).take(visible_rows) {
                let is_selected = i == self.agenda_cursor;
                let is_editing = is_selected && self.agenda_editing;

                let date_text = if is_editing && self.agenda_edit_col == AgendaColumn::Date {
                    format!("{:<width$}", format!("{}|", self.agenda_input), width = date_w)
                } else {
                    format!("{:<width$}", format_short_date(&agenda.date.format("%Y-%m-%d").to_string()), width = date_w)
                };

                let title_text = if is_editing && self.agenda_edit_col == AgendaColumn::Title {
                    format!("{:<width$}", truncate(&format!("{}|", self.agenda_input), title_w), width = title_w)
                } else {
                    format!("{:<width$}", truncate(&agenda.title, title_w), width = title_w)
                };

                let tags_str = agenda.refs.tags.iter()
                    .map(|t| format!("#{}", t))
                    .collect::<Vec<_>>()
                    .join(" ");
                let tags_text = if is_editing && self.agenda_edit_col == AgendaColumn::Tags {
                    truncate(&format!("{}|", self.agenda_input), tags_w)
                } else {
                    truncate(&tags_str, tags_w)
                };

                let date_style = if is_editing && self.agenda_edit_col == AgendaColumn::Date {
                    theme.column_focus
                } else if is_selected && focused && self.agenda_edit_col == AgendaColumn::Date {
                    theme.column_focus
                } else {
                    theme.date
                };

                let title_style_cell = if is_editing && self.agenda_edit_col == AgendaColumn::Title {
                    theme.column_focus
                } else if is_selected && focused && self.agenda_edit_col == AgendaColumn::Title {
                    theme.column_focus
                } else {
                    theme.title.remove_modifier(Modifier::BOLD)
                };

                let tags_style = if is_editing && self.agenda_edit_col == AgendaColumn::Tags {
                    theme.column_focus
                } else if is_selected && focused && self.agenda_edit_col == AgendaColumn::Tags {
                    theme.column_focus
                } else {
                    theme.topic
                };

                let spans = vec![
                    Span::styled(" ", theme.dim),
                    Span::styled(date_text, date_style),
                    Span::styled(" ", theme.dim),
                    Span::styled(title_text, title_style_cell),
                    Span::styled(" ", theme.dim),
                    Span::styled(tags_text, tags_style),
                ];

                let mut line = Line::from(spans);
                if is_selected && focused {
                    line = line.style(theme.selected);
                } else if is_selected {
                    line = line.style(theme.row_gray);
                }
                lines.push(line);
            }
        }

        let para = Paragraph::new(lines);
        frame.render_widget(para, inner);
    }

    fn draw_timeline(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let focused = self.focus == PeopleFocus::Timeline;
        let (title_style, border_style) = self.section_styles(focused, theme);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Span::styled(" Timeline ", title_style));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let visible_rows = inner.height as usize;
        let scroll = if self.timeline_cursor >= visible_rows {
            self.timeline_cursor - visible_rows + 1
        } else {
            0
        };

        let mut lines: Vec<Line> = Vec::new();

        if self.timeline.is_empty() {
            lines.push(Line::from(Span::styled(
                " No references found",
                theme.dim,
            )));
        } else {
            for (i, item) in self.timeline.iter().enumerate().skip(scroll).take(visible_rows) {
                let is_selected = i == self.timeline_cursor;

                let title_text = if item.private && !self.revealed.contains(&item.id) {
                    mask_private(&item.title, 8)
                } else {
                    let max_w = inner.width.saturating_sub(12) as usize;
                    truncate(&item.title, max_w)
                };

                let mut spans = Vec::new();

                // Date
                let date_display = format_short_date(&item.date);
                spans.push(Span::styled(
                    format!(" {} ", date_display),
                    theme.date,
                ));

                // Icon
                spans.push(Span::styled(
                    format!("{} ", item.icon()),
                    theme.dim,
                ));

                // Title
                let title_style = if item.private && !self.revealed.contains(&item.id) {
                    theme.private
                } else if item.done {
                    theme.status_done
                } else {
                    theme.title.remove_modifier(Modifier::BOLD)
                };
                spans.push(Span::styled(title_text, title_style));

                let mut line = Line::from(spans);
                if is_selected && self.focus == PeopleFocus::Timeline {
                    line = line.style(theme.selected);
                } else if is_selected {
                    line = line.style(theme.row_gray);
                }
                lines.push(line);
            }
        }

        let para = Paragraph::new(lines);
        frame.render_widget(para, inner);
    }
}

// ── Helpers ────────────────────────────────────────────────────────

fn format_short_date(date_str: &str) -> String {
    crate::util::date_format::format_date_str(date_str)
}
