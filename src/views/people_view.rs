use std::collections::{HashMap, HashSet};
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
use crate::util::TextInput;
use super::{icons, mask_private, truncate, View};

// ── Focus enum ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeopleFocus {
    Sidebar,
    Agendas,
    Timeline,
}

impl PeopleFocus {
    fn next(self) -> Self {
        match self {
            Self::Sidebar => Self::Agendas,
            Self::Agendas => Self::Timeline,
            Self::Timeline => Self::Sidebar,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::Sidebar => Self::Timeline,
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
    Refs,
}

impl AgendaColumn {
    fn next(self) -> Self {
        match self {
            Self::Date  => Self::Title,
            Self::Title => Self::Tags,
            Self::Tags  => Self::Refs,
            Self::Refs  => Self::Date,
        }
    }
    fn prev(self) -> Self {
        match self {
            Self::Date  => Self::Refs,
            Self::Title => Self::Date,
            Self::Tags  => Self::Title,
            Self::Refs  => Self::Tags,
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
    meta_input: TextInput,
    meta_cursor: usize,
    meta_field: MetaField,
    /// Ordered list of metadata keys for the selected person (for stable cursor).
    meta_keys: Vec<MetaField>,

    // Two-field editing in the popup (key + value separately)
    meta_edit_in_key: bool,
    meta_key_input: TextInput,
    // Add mode: new row at bottom of popup
    meta_add_mode: bool,
    meta_add_in_key: bool,
    meta_add_key: TextInput,

    // 1:1 Agendas
    agendas: Vec<Agenda>,
    agenda_backlink_counts: HashMap<String, usize>,
    agenda_cursor: usize,
    agenda_editing: bool,
    agenda_edit_col: AgendaColumn,
    agenda_input: TextInput,
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

    // Sidebar counts: slug → (tasks, notes, agendas)
    counts: HashMap<String, (usize, usize, usize)>,

    // Visibility
    show_archived: bool,

    // Global tag filter
    tag_filter: Option<String>,

    /// Pending delete confirmation: (focus context, display title, closure data).
    confirm_delete: Option<(PeopleFocus, String)>,

    // Slug editing (new person creation prompt)
    slug_editing: bool,
    slug_input: TextInput,

    // Rename slug (existing person)
    rename_editing: bool,
    rename_input: String,
    rename_cursor: usize,
    rename_error: Option<String>,
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
            meta_input: TextInput::new(),
            meta_cursor: 0,
            meta_field: MetaField(String::new()),
            meta_keys: Vec::new(),
            meta_edit_in_key: false,
            meta_key_input: TextInput::new(),
            meta_add_mode: false,
            meta_add_in_key: true,
            meta_add_key: TextInput::new(),
            agendas: Vec::new(),
            agenda_backlink_counts: HashMap::new(),
            agenda_cursor: 0,
            agenda_editing: false,
            agenda_edit_col: AgendaColumn::Date,
            agenda_input: TextInput::new(),
            agenda_input_cursor: 0,
            agenda_sort_column: None,
            agenda_sort_direction: None,
            timeline: Vec::new(),
            timeline_cursor: 0,
            content_width: 80,
            content_height: 24,
            revealed: HashSet::new(),
            counts: HashMap::new(),
            show_archived: false,
            tag_filter: None,
            confirm_delete: None,
            slug_editing: false,
            slug_input: TextInput::new(),
            rename_editing: false,
            rename_input: String::new(),
            rename_cursor: 0,
            rename_error: None,
        }
    }

    // ── Data loading ─────────────────────────────────────────────

    /// Full reload: re-fetches and re-sorts the people list, then refreshes the
    /// right-side panels.  Call only on tab entry or when the list itself changes
    /// (archive toggle, pin, rename).
    pub fn on_tab_entered(&mut self) {
        self.reload();
    }

    /// Refresh only the detail panels (agendas + timeline) for the currently
    /// selected person, without re-sorting `self.people`.  Call this after
    /// mutations that change agenda/metadata data but should not move the cursor.
    fn reload_detail(&mut self) {
        self.reload_right_side();
    }

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

        // Sort people: pinned first (by frecency within group), then unpinned (by frecency)
        let frecency = self.store.person_frecency_scores();
        self.people.sort_by(|a, b| {
            let sa = frecency.get(&a.slug).copied().unwrap_or(0.0);
            let sb = frecency.get(&b.slug).copied().unwrap_or(0.0);
            b.pinned.cmp(&a.pinned)
                .then_with(|| sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal))
                .then_with(|| a.slug.cmp(&b.slug))
        });

        // Clamp cursor
        if !self.people.is_empty() && self.left_cursor >= self.people.len() {
            self.left_cursor = self.people.len() - 1;
        }

        self.reload_counts();
        self.reload_right_side();
    }

    fn reload_counts(&mut self) {
        self.counts.clear();
        for person in &self.people {
            let refs = self.store.get_memory(&person.slug);
            let mut tasks = 0usize;
            let mut notes = 0usize;
            let mut agendas = 0usize;
            for eref in &refs {
                match eref.kind {
                    EntityKind::Task => tasks += 1,
                    EntityKind::Note => notes += 1,
                    EntityKind::Agenda => agendas += 1,
                    _ => {}
                }
            }
            self.counts.insert(person.slug.clone(), (tasks, notes, agendas));
        }
    }

    fn create_person(&mut self) {
        let default_slug = format!("person-{}", &crate::domain::new_id()[..8]);
        self.slug_editing = true;
        self.slug_input.set(default_slug);
    }

    fn confirm_slug_and_create(&mut self) {
        self.slug_editing = false;

        // Normalize: lowercase, non-alphanumeric → hyphen, trim leading/trailing hyphens
        let raw = self.slug_input.value().trim().to_owned();
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
        if let Err(e) = self.store.save_person(&person) {
            eprintln!("Failed to save person: {e}");
        }
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
        self.meta_add_key.set("name");
        self.meta_input.clear();
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
        self.agenda_backlink_counts = self.agendas
            .iter()
            .map(|a| (a.id.clone(), self.store.get_backlinks("agenda", &a.id).len()))
            .collect();
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
            (Some(AgendaColumn::Refs), Some(SortDirection::Ascending)) => {
                self.agendas.sort_by(|a, b| {
                    let ca = a.refs.tasks.len() + a.refs.notes.len() + a.refs.agendas.len()
                        + self.agenda_backlink_counts.get(&a.id).copied().unwrap_or(0);
                    let cb = b.refs.tasks.len() + b.refs.notes.len() + b.refs.agendas.len()
                        + self.agenda_backlink_counts.get(&b.id).copied().unwrap_or(0);
                    ca.cmp(&cb)
                });
            }
            (Some(AgendaColumn::Refs), Some(SortDirection::Descending)) => {
                self.agendas.sort_by(|a, b| {
                    let ca = a.refs.tasks.len() + a.refs.notes.len() + a.refs.agendas.len()
                        + self.agenda_backlink_counts.get(&a.id).copied().unwrap_or(0);
                    let cb = b.refs.tasks.len() + b.refs.notes.len() + b.refs.agendas.len()
                        + self.agenda_backlink_counts.get(&b.id).copied().unwrap_or(0);
                    cb.cmp(&ca)
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

        self.meta_key_input.set(key.clone());
        self.meta_input.set(value);
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
        let new_key = self.meta_key_input.value().trim().to_owned();
        let val = self.meta_input.value().trim().to_owned();

        // Remove old key regardless of rename
        person.metadata.remove(&old_key);

        // Re-insert only if both key and value are non-empty
        if !new_key.is_empty() && !val.is_empty() {
            person.metadata.insert(new_key.clone(), val.clone());
        }

        if let Err(e) = self.store.save_person(person) {
            eprintln!("Failed to save person: {e}");
        }
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

        // Only rename auto-generated slugs.
        if !old_slug.starts_with("person-") {
            return;
        }

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

        // Attempt atomic rename; silently skip on collision (slug already taken).
        if self.store.rename_person(&old_slug, &new_slug).is_ok() {
            self.reload();
            if let Some(idx) = self.people.iter().position(|p| p.slug == new_slug) {
                self.left_cursor = idx;
                self.reload_right_side();
            }
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

        if let Err(e) = self.store.save_person(person) {
            eprintln!("Failed to save person: {e}");
        }
        self.reload_meta_keys();
    }

    fn add_meta_field(&mut self) {
        self.meta_add_mode = true;
        self.meta_add_in_key = true;
        self.meta_add_key.clear();
        self.meta_input.clear();
        self.editing_metadata = true;
    }

    fn save_new_meta_field(&mut self) {
        self.meta_add_mode = false;
        self.editing_metadata = false;

        let key = self.meta_add_key.value().trim().to_owned();
        let val = self.meta_input.value().trim().to_owned();

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
        if let Err(e) = self.store.save_person(person) {
            eprintln!("Failed to save person: {e}");
        }
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
        if let Err(e) = self.store.save_agenda(&agenda) {
            return Some(AppMessage::Error(format!("Failed to save agenda: {e}")));
        }
        self.reload_agendas();
        // Open the new agenda in the inline editor
        Some(AppMessage::OpenInlineEditor { kind: EntityKind::Agenda, id })
    }

    fn delete_agenda(&mut self) {
        if let Some(agenda) = self.agendas.get(self.agenda_cursor).cloned() {
            if let Err(e) = self.store.delete_agenda(&agenda.id) {
                eprintln!("Failed to delete agenda: {e}");
            }
            self.reload_agendas();
            if self.agenda_cursor > 0 && self.agenda_cursor >= self.agendas.len() {
                self.agenda_cursor = self.agendas.len().saturating_sub(1);
            }
        }
    }

    fn start_agenda_edit(&mut self) {
        let Some(agenda) = self.agendas.get(self.agenda_cursor) else { return; };
        self.agenda_editing = true;
        let val = match self.agenda_edit_col {
            AgendaColumn::Date  => agenda.date.format("%Y-%m-%d").to_string(),
            AgendaColumn::Title => agenda.title.clone(),
            AgendaColumn::Tags  => agenda.refs.tags.iter()
                .map(|t| format!("#{}", t))
                .collect::<Vec<_>>()
                .join(" "),
            AgendaColumn::Refs  => return, // read-only
        };
        self.agenda_input_cursor = val.len();
        self.agenda_input.set(val);
    }

    fn save_agenda_edit(&mut self) {
        self.agenda_editing = false;
        let Some(agenda) = self.agendas.get_mut(self.agenda_cursor) else {
            self.agenda_input.clear();
            return;
        };

        match self.agenda_edit_col {
            AgendaColumn::Date => {
                let trimmed = self.agenda_input.value().trim();
                if let Ok(parsed_date) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
                    agenda.date = parsed_date;
                }
            }
            AgendaColumn::Title => {
                let trimmed = self.agenda_input.value().trim().to_owned();
                if !trimmed.is_empty() {
                    agenda.title = trimmed;
                }
            }
            AgendaColumn::Tags => {
                agenda.refs.tags = self.agenda_input.value().split_whitespace()
                    .map(|s| s.trim_start_matches('#').to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            AgendaColumn::Refs => {
                // Read-only; ignore
                self.agenda_input.clear();
                return;
            }
        }

        agenda.updated_at = chrono::Utc::now();
        if let Err(e) = self.store.save_agenda(agenda) {
            eprintln!("Failed to save agenda: {e}");
        }
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

        if self.rename_editing {
            return self.handle_rename_key(*code);
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
                // Only refresh detail panels; the people list sort is preserved
                // so that the cursor does not jump after agenda/metadata mutations.
                // A full re-sort is triggered by on_tab_entered() when the tab is
                // actually entered.
                self.reload_detail();
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
                Constraint::Percentage(25),
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

        if self.rename_editing {
            self.draw_rename_popup(frame, main_area, theme);
        }
    }

    fn captures_input(&self) -> bool {
        self.slug_editing || self.rename_editing || self.show_meta_popup || self.editing_metadata || self.agenda_editing || self.confirm_delete.is_some()
    }
}

impl PeopleView {
    /// Navigate to a specific agenda by person slug and agenda ID.
    /// Selects the person, switches to the Agendas pane, and scrolls to the agenda.
    pub fn navigate_to_agenda(&mut self, person_slug: &str, agenda_id: &str) {
        self.reload();
        if let Some(idx) = self.people.iter().position(|p| p.slug == person_slug) {
            self.left_cursor = idx;
            self.reload_right_side();
            self.focus = PeopleFocus::Agendas;
            if let Some(pos) = self.agendas.iter().position(|a| a.id == agenda_id) {
                self.agenda_cursor = pos;
            }
        }
    }

    /// Return the focused agenda ID when the Agendas pane is active.
    pub fn focused_entity_id(&self) -> Option<(crate::domain::EntityKind, String)> {
        if self.focus == PeopleFocus::Agendas {
            self.agendas.get(self.agenda_cursor)
                .map(|a| (crate::domain::EntityKind::Agenda, a.id.clone()))
        } else {
            None
        }
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
                if self.focus == PeopleFocus::Agendas {
                    self.agenda_edit_col = AgendaColumn::Title;
                }
                None
            }
            KeyCode::BackTab => {
                self.focus = self.focus.prev();
                if self.focus == PeopleFocus::Agendas {
                    self.agenda_edit_col = AgendaColumn::Title;
                }
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
                        if let Some(person) = self.people.get(self.left_cursor) {
                            self.rename_input = person.slug.clone();
                            self.rename_cursor = self.rename_input.len();
                            self.rename_error = None;
                            self.rename_editing = true;
                        }
                        None
                    }
                    PeopleFocus::Agendas => {
                        if !self.agendas.is_empty() {
                            if self.agenda_edit_col == AgendaColumn::Refs {
                                let agenda = &self.agendas[self.agenda_cursor];
                                return Some(AppMessage::OpenLinkOverlay {
                                    source_kind: EntityKind::Agenda,
                                    source_id: agenda.id.clone(),
                                });
                            } else if self.agenda_edit_col == AgendaColumn::Date {
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
                    PeopleFocus::Agendas => {
                        self.create_agenda()
                    }
                    _ => None,
                }
            }
            KeyCode::Char('d') => {
                match self.focus {
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
                    PeopleFocus::Agendas => {
                        if let Some(agenda) = self.agendas.get(self.agenda_cursor) {
                            if self.agenda_edit_col == AgendaColumn::Refs {
                                return Some(AppMessage::OpenLinkOverlay {
                                    source_kind: EntityKind::Agenda,
                                    source_id: agenda.id.clone(),
                                });
                            }
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
                        if let Err(e) = self.store.save_person(&updated) {
                            return Some(AppMessage::Error(format!("Failed to save person: {e}")));
                        }
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
                        if let Err(e) = self.store.save_person(&updated) {
                            return Some(AppMessage::Error(format!("Failed to save person: {e}")));
                        }
                        self.reload();
                    }
                }
                None
            }
            KeyCode::Char('x') => {
                if self.focus == PeopleFocus::Agendas {
                    if let Some(agenda) = self.agendas.get(self.agenda_cursor) {
                        return Some(AppMessage::OpenRefExplorer {
                            kind: EntityKind::Agenda,
                            id: agenda.id.clone(),
                            title: agenda.title.clone(),
                        });
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
        let input_display = format!("@{}|", self.slug_input.value());
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

    fn handle_rename_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        match code {
            KeyCode::Enter => {
                let new_slug = self.rename_input.trim()
                    .to_lowercase()
                    .chars()
                    .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
                    .collect::<String>()
                    .trim_matches('-')
                    .to_string();

                if new_slug.is_empty() {
                    self.rename_error = Some("Slug cannot be empty".into());
                    return None;
                }

                let old_slug = match self.people.get(self.left_cursor) {
                    Some(p) => p.slug.clone(),
                    None => { self.rename_editing = false; return None; }
                };

                if new_slug == old_slug {
                    self.rename_editing = false;
                    self.rename_input.clear();
                    self.rename_cursor = 0;
                    self.rename_error = None;
                    return None;
                }

                match self.store.rename_person(&old_slug, &new_slug) {
                    Ok(()) => {
                        self.rename_editing = false;
                        self.rename_input.clear();
                        self.rename_cursor = 0;
                        self.rename_error = None;
                        self.reload();
                        if let Some(idx) = self.people.iter().position(|p| p.slug == new_slug) {
                            self.left_cursor = idx;
                            self.reload_right_side();
                        }
                        Some(AppMessage::Reload)
                    }
                    Err(_) => {
                        self.rename_error = Some(format!("@{} already exists", new_slug));
                        None
                    }
                }
            }
            KeyCode::Esc => {
                self.rename_editing = false;
                self.rename_input.clear();
                self.rename_cursor = 0;
                self.rename_error = None;
                None
            }
            KeyCode::Backspace => {
                if self.rename_cursor > 0 {
                    let mut prev = self.rename_cursor - 1;
                    while prev > 0 && !self.rename_input.is_char_boundary(prev) {
                        prev -= 1;
                    }
                    self.rename_input.drain(prev..self.rename_cursor);
                    self.rename_cursor = prev;
                }
                self.rename_error = None;
                None
            }
            KeyCode::Left => {
                if self.rename_cursor > 0 {
                    let mut prev = self.rename_cursor - 1;
                    while prev > 0 && !self.rename_input.is_char_boundary(prev) {
                        prev -= 1;
                    }
                    self.rename_cursor = prev;
                }
                None
            }
            KeyCode::Right => {
                if self.rename_cursor < self.rename_input.len() {
                    let mut next = self.rename_cursor + 1;
                    while next < self.rename_input.len() && !self.rename_input.is_char_boundary(next) {
                        next += 1;
                    }
                    self.rename_cursor = next;
                }
                None
            }
            KeyCode::Home => {
                self.rename_cursor = 0;
                None
            }
            KeyCode::End => {
                self.rename_cursor = self.rename_input.len();
                None
            }
            KeyCode::Char(c) => {
                self.rename_input.insert(self.rename_cursor, c);
                self.rename_cursor += c.len_utf8();
                self.rename_error = None;
                None
            }
            _ => None,
        }
    }

    fn draw_rename_popup(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let Some(person) = self.people.get(self.left_cursor) else { return; };

        let popup_h = if self.rename_error.is_some() { 7u16 } else { 5u16 };
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
            .title(Span::styled(format!(" Rename @{} ", person.slug), theme.title));

        let inner = block.inner(popup_rect);
        frame.render_widget(block, popup_rect);

        let max_w = inner.width.saturating_sub(10) as usize;
        let before = &self.rename_input[..self.rename_cursor];
        let after = &self.rename_input[self.rename_cursor..];
        let input_display = format!("@{}|{}", before, after);
        let input_display = truncate(&input_display, max_w);

        let mut lines = vec![
            Line::from(vec![
                Span::styled("  Slug  ", theme.dim),
                Span::styled(input_display, theme.person),
            ]),
            Line::from(""),
            Line::from(Span::styled("  Enter to confirm · Esc to cancel", theme.dim)),
        ];

        if let Some(ref err) = self.rename_error {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  {}", truncate(err, inner.width.saturating_sub(4) as usize)),
                theme.error,
            )));
        }

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
                            self.confirm_delete = Some((PeopleFocus::Sidebar, key.clone()));
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
                        PeopleFocus::Sidebar => {
                            self.delete_meta_field();
                        }
                        PeopleFocus::Agendas => {
                            self.delete_agenda();
                        }
                        PeopleFocus::Timeline => {
                            if let Some(item) = self.timeline.get(self.timeline_cursor) {
                                let item_id = item.id.clone();
                                let item_kind = item.kind.clone();
                                let del_result = match item_kind {
                                    EntityKind::Task => self.store.delete_task(&item_id),
                                    EntityKind::Note => self.store.delete_note(&item_id),
                                    EntityKind::Agenda => self.store.delete_agenda(&item_id),
                                    _ => Ok(()),
                                };
                                if let Err(e) = del_result {
                                    return Some(AppMessage::Error(format!("Failed to delete: {e}")));
                                }
                                self.reload();
                            }
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
                theme.accent,
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

        // Show per-person counts (tasks/notes/agendas) when the sidebar is wide enough.
        // Format: `󰄱N 󰈙N 󰏪N` — 11 chars fixed width + 1 leading space = 12 reserved.
        const COUNTS_WIDTH: u16 = 12;
        const MIN_WIDTH_FOR_COUNTS: u16 = 26;
        let show_counts = inner.width >= MIN_WIDTH_FOR_COUNTS;

        for (i, person) in self.people.iter().enumerate().skip(scroll).take(visible_rows) {
            let is_selected = i == self.left_cursor;
            let display = self.person_display(person);

            let prefix = if person.archived {
                format!("{} ", icons::ARCHIVE)
            } else if person.pinned {
                format!("{} ", icons::PIN)
            } else {
                "  ".to_string()
            };
            let prefix_chars = prefix.chars().count() as u16;
            let prefix_style = if person.archived { theme.dim } else { theme.error };
            let name_style = if person.archived { theme.dim } else { theme.person };

            let spans = if show_counts {
                let reserved = prefix_chars + COUNTS_WIDTH;
                let name_max_w = inner.width.saturating_sub(reserved) as usize;
                let name_str = truncate(&display, name_max_w);
                // Pad name to fill the column so counts align vertically
                let name_chars = name_str.chars().count();
                let pad = name_max_w.saturating_sub(name_chars);
                let padded = format!("{}{}", name_str, " ".repeat(pad));

                let (t, n, a) = self.counts.get(&person.slug).copied().unwrap_or((0, 0, 0));
                let counts_str = format!(" {}{:2} {}{:2} {}{:2}",
                    icons::TASK, t.min(99),
                    icons::NOTE, n.min(99),
                    icons::AGENDA, a.min(99),
                );
                vec![
                    Span::styled(prefix, prefix_style),
                    Span::styled(padded, name_style),
                    Span::styled(counts_str, theme.dim),
                ]
            } else {
                let max_name_w = inner.width.saturating_sub(prefix_chars + 1) as usize;
                let display = truncate(&display, max_name_w);
                vec![
                    Span::styled(prefix, prefix_style),
                    Span::styled(display, name_style),
                ]
            };

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
            .title(Span::styled(format!(" @{} ", person.slug), theme.title));

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
                    let s = format!("{}{}", self.meta_key_input.value(),
                        if self.meta_edit_in_key { "|" } else { "" });
                    format!("{:<width$}", truncate(&s, max_key_w), width = max_key_w)
                } else {
                    format!("{:<width$}", truncate(key, max_key_w), width = max_key_w)
                };

                // Value display: show text cursor when actively editing the value field
                let val_display = if is_editing {
                    let s = format!("{}{}", self.meta_input.value(),
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
                let key_text = format!("{}{}", self.meta_add_key.value(),
                    if self.meta_add_in_key { "|" } else { "" });
                let val_text = format!("{}{}", self.meta_input.value(),
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
        let refs_w: usize = 8;
        let title_w = (inner.width as usize).saturating_sub(date_w + tags_w + refs_w + 4); // 1 leading + 3 separators

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
        let refs_arrow = match (self.agenda_sort_column, self.agenda_sort_direction) {
            (Some(AgendaColumn::Refs), Some(SortDirection::Ascending)) => " \u{2191}",
            (Some(AgendaColumn::Refs), Some(SortDirection::Descending)) => " \u{2193}",
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
            Span::styled(format!("{:<width$}", format!("TAGS{}", tags_arrow), width = tags_w), col_header_style),
            Span::styled(" ", theme.dim),
            Span::styled(format!("REFS{}", refs_arrow), col_header_style),
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
                    format!("{:<width$}", format!("{}|", self.agenda_input.value()), width = date_w)
                } else {
                    format!("{:<width$}", format_short_date(&agenda.date.format("%Y-%m-%d").to_string()), width = date_w)
                };

                let title_text = if is_editing && self.agenda_edit_col == AgendaColumn::Title {
                    format!("{:<width$}", truncate(&format!("{}|", self.agenda_input.value()), title_w), width = title_w)
                } else {
                    format!("{:<width$}", truncate(&agenda.title, title_w), width = title_w)
                };

                let tags_str = agenda.refs.tags.iter()
                    .map(|t| format!("#{}", t))
                    .collect::<Vec<_>>()
                    .join(" ");
                let tags_text = if is_editing && self.agenda_edit_col == AgendaColumn::Tags {
                    truncate(&format!("{}|", self.agenda_input.value()), tags_w)
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

                let out_count = agenda.refs.tasks.len() + agenda.refs.notes.len() + agenda.refs.agendas.len();
                let back_count = self.agenda_backlink_counts.get(&agenda.id).copied().unwrap_or(0);
                let refs_text = match (out_count > 0, back_count > 0) {
                    (true, true)   => format!("󰌷{} 󱞥{}", out_count, back_count),
                    (true, false)  => format!("󰌷{}", out_count),
                    (false, true)  => format!("󱞥{}", back_count),
                    (false, false) => String::new(),
                };
                let has_refs = out_count > 0 || back_count > 0;
                let refs_cell_style = if is_selected && focused && self.agenda_edit_col == AgendaColumn::Refs {
                    theme.column_focus
                } else if has_refs {
                    theme.accent
                } else {
                    theme.dim
                };

                let spans = vec![
                    Span::styled(" ", theme.dim),
                    Span::styled(date_text, date_style),
                    Span::styled(" ", theme.dim),
                    Span::styled(title_text, title_style_cell),
                    Span::styled(" ", theme.dim),
                    Span::styled(format!("{:<width$}", tags_text, width = tags_w), tags_style),
                    Span::styled(" ", theme.dim),
                    Span::styled(format!("{:<width$}", refs_text, width = refs_w), refs_cell_style),
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
