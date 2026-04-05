use std::collections::HashSet;
use std::sync::Arc;

use chrono::{Local, NaiveDate};
use crossterm::event::{Event, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::message::AppMessage;
use crate::app::theme::Theme;
use crate::domain::{EntityKind, TaskStatus};
use crate::store::Store;
use super::{icons, mask_private, truncate, View};

// ── AgendaItem ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct AgendaItem {
    id: String,
    kind: EntityKind,
    title: String,
    due_date: Option<NaiveDate>,
    due_time: Option<String>,
    done: bool,
    private: bool,
}

impl AgendaItem {
    fn icon(&self) -> &'static str {
        match self.kind {
            EntityKind::Task => icons::TASK,
            EntityKind::Note => icons::NOTE,
            EntityKind::Person => icons::MEMORY,
            EntityKind::Agenda => icons::AGENDA,
            _ => icons::TASK,
        }
    }
}

// ── Quadrant enum ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quadrant {
    Upcoming,
    Overdue,
    Recent,
    Pinned,
}

impl Quadrant {
    fn title(self) -> &'static str {
        match self {
            Quadrant::Upcoming => " Upcoming ",
            Quadrant::Overdue => " Overdue ",
            Quadrant::Recent => " Recent ",
            Quadrant::Pinned => " Pinned ",
        }
    }
}

// ── FlatIndex ─────────────────────────────────────────────────────
// The flat cursor spans all four quadrants so the user can navigate
// with j/k across the entire dashboard.

#[derive(Debug, Clone)]
struct FlatEntry {
    quadrant: Quadrant,
    item: AgendaItem,
}

// ── Dashboard ─────────────────────────────────────────────────────

pub struct Dashboard {
    store: Arc<dyn Store>,
    upcoming: Vec<AgendaItem>,
    overdue: Vec<AgendaItem>,
    recent: Vec<AgendaItem>,
    pinned: Vec<AgendaItem>,
    flat: Vec<FlatEntry>,
    cursor: usize,
    revealed: HashSet<String>,
    content_width: u16,
    content_height: u16,
    tag_filter: Option<String>,
    /// Index of entry awaiting delete confirmation.
    confirm_delete: Option<usize>,
}

impl Dashboard {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self {
            store,
            upcoming: Vec::new(),
            overdue: Vec::new(),
            recent: Vec::new(),
            pinned: Vec::new(),
            flat: Vec::new(),
            cursor: 0,
            revealed: HashSet::new(),
            content_width: 0,
            content_height: 0,
            tag_filter: None,
            confirm_delete: None,
        }
    }

    // ── Data loading ──────────────────────────────────────────────

    fn reload(&mut self) {
        self.upcoming.clear();
        self.overdue.clear();
        self.recent.clear();
        self.pinned.clear();

        let today = Local::now().date_naive();

        // Tasks
        if let Ok(tasks) = self.store.list_tasks() {
            for t in tasks {
                // Apply global tag filter
                if let Some(ref tag) = self.tag_filter {
                    if !t.refs.tags.iter().any(|tg| tg == tag) {
                        continue;
                    }
                }
                let is_done = t.status == TaskStatus::Done;
                if t.archived {
                    continue;
                }

                let overdue = is_date_overdue(&t.due_date, today);
                let item = AgendaItem {
                    id: t.id.clone(),
                    kind: EntityKind::Task,
                    title: t.title.clone(),
                    due_date: t.due_date.clone(),
                    due_time: t.due_time.clone(),
                    done: is_done,
                    private: t.private,
                };

                if t.pinned {
                    self.pinned.push(item.clone());
                }
                if overdue && !is_done {
                    self.overdue.push(item);
                } else if is_done {
                    self.recent.push(item);
                } else if t.due_date.is_some() {
                    self.upcoming.push(item);
                } else {
                    // No due date, not done -- put in recent by creation
                    self.recent.push(item);
                }
            }
        }

        // Pinned notes
        if let Ok(notes) = self.store.list_notes() {
            for n in notes {
                if let Some(ref tag) = self.tag_filter {
                    if !n.refs.tags.iter().any(|tg| tg == tag) {
                        continue;
                    }
                }
                if n.pinned {
                    self.pinned.push(AgendaItem {
                        id: n.id.clone(),
                        kind: EntityKind::Note,
                        title: n.title.clone(),
                        due_date: None,
                        due_time: None,
                        done: false,
                        private: n.private,
                    });
                }
            }
        }

        // Pinned people
        if let Ok(people) = self.store.list_persons() {
            for p in people {
                if p.pinned {
                    self.pinned.push(AgendaItem {
                        id: p.slug.clone(),
                        kind: EntityKind::Person,
                        title: p.display_name(),
                        due_date: None,
                        due_time: None,
                        done: false,
                        private: false,
                    });
                }
            }
        }

        // Sort: upcoming by due_date ascending, overdue by due_date descending,
        // recent by most recent first
        self.upcoming.sort_by(|a, b| a.due_date.cmp(&b.due_date));
        self.overdue.sort_by(|a, b| b.due_date.cmp(&a.due_date));
        // recent: just keep insertion order (newest tasks first from store)

        // Build flat index
        self.rebuild_flat();

        // Clamp cursor
        if !self.flat.is_empty() && self.cursor >= self.flat.len() {
            self.cursor = self.flat.len() - 1;
        }
    }

    fn rebuild_flat(&mut self) {
        self.flat.clear();
        for item in &self.upcoming {
            self.flat.push(FlatEntry {
                quadrant: Quadrant::Upcoming,
                item: item.clone(),
            });
        }
        for item in &self.overdue {
            self.flat.push(FlatEntry {
                quadrant: Quadrant::Overdue,
                item: item.clone(),
            });
        }
        for item in &self.recent {
            self.flat.push(FlatEntry {
                quadrant: Quadrant::Recent,
                item: item.clone(),
            });
        }
        for item in &self.pinned {
            self.flat.push(FlatEntry {
                quadrant: Quadrant::Pinned,
                item: item.clone(),
            });
        }
    }

    fn current_entry(&self) -> Option<&FlatEntry> {
        self.flat.get(self.cursor)
    }

    // ── Quadrant items for a specific pane ────────────────────────

    fn items_for(&self, q: Quadrant) -> Vec<&FlatEntry> {
        self.flat.iter().filter(|e| e.quadrant == q).collect()
    }

    fn flat_index_of_quadrant_start(&self, q: Quadrant) -> Option<usize> {
        self.flat.iter().position(|e| e.quadrant == q)
    }
}

// ── View trait impl ───────────────────────────────────────────────

impl View for Dashboard {
    fn handle_event(&mut self, event: &Event) -> Option<AppMessage> {
        let Event::Key(KeyEvent { code, .. }) = event else {
            return None;
        };

        if self.confirm_delete.is_some() {
            return self.handle_confirm_delete_key(*code);
        }

        if self.flat.is_empty() {
            return None;
        }

        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if self.cursor + 1 < self.flat.len() {
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
            KeyCode::Tab => {
                // Jump to first item of next quadrant
                if let Some(current) = self.flat.get(self.cursor) {
                    let current_q = current.quadrant;
                    // Find first item after current cursor that's in a different quadrant
                    if let Some(next_idx) = self.flat.iter().enumerate()
                        .skip(self.cursor + 1)
                        .find(|(_, e)| e.quadrant != current_q)
                        .map(|(i, _)| i)
                    {
                        self.cursor = next_idx;
                    } else {
                        // Wrap to beginning
                        self.cursor = 0;
                    }
                }
                None
            }
            KeyCode::BackTab => {
                // Jump to first item of previous quadrant
                if let Some(current) = self.flat.get(self.cursor) {
                    let current_q = current.quadrant;
                    // Find the start of the current quadrant
                    let current_q_start = self.flat.iter().position(|e| e.quadrant == current_q).unwrap_or(0);
                    if current_q_start > 0 {
                        // Go to start of previous quadrant
                        let prev_q = self.flat[current_q_start - 1].quadrant;
                        let prev_start = self.flat.iter().position(|e| e.quadrant == prev_q).unwrap_or(0);
                        self.cursor = prev_start;
                    } else {
                        // Wrap to last quadrant
                        if let Some(last_q) = self.flat.last().map(|e| e.quadrant) {
                            let last_start = self.flat.iter().position(|e| e.quadrant == last_q).unwrap_or(0);
                            self.cursor = last_start;
                        }
                    }
                }
                None
            }
            KeyCode::Char('g') => {
                self.cursor = 0;
                None
            }
            KeyCode::Char('G') => {
                if !self.flat.is_empty() {
                    self.cursor = self.flat.len() - 1;
                }
                None
            }
            KeyCode::Enter => {
                if let Some(entry) = self.current_entry() {
                    let item_id = entry.item.id.clone();
                    let item_kind = entry.item.kind.clone();
                    let item_private = entry.item.private;
                    if item_private {
                        // Toggle reveal
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
                        _ => {}
                    }
                }
                None
            }
            KeyCode::Char('e') => {
                if let Some(entry) = self.current_entry() {
                    let item = &entry.item;
                    return Some(AppMessage::EditEntity {
                        kind: item.kind.clone(),
                        id: item.id.clone(),
                    });
                }
                None
            }
            KeyCode::Char('d') => {
                if self.current_entry().is_some() {
                    self.confirm_delete = Some(self.cursor);
                }
                None
            }
            _ => None,
        }
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
        if self.flat.is_empty() {
            let empty_lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "Nothing on the agenda today",
                    theme.title,
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Press 's' to capture, 'T' for tasks, 'n' to create",
                    theme.dim,
                )),
            ];
            let p = Paragraph::new(empty_lines)
                .alignment(ratatui::layout::Alignment::Center);
            frame.render_widget(p, area);
            return;
        }

        // Reserve bottom row for confirm bar if needed
        let (main_area, confirm_area) = if self.confirm_delete.is_some() {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(area);
            (chunks[0], Some(chunks[1]))
        } else {
            (area, None)
        };

        // 2x2 grid layout
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_area);

        let top_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[0]);

        let bot_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[1]);

        let focused_q = self.flat.get(self.cursor).map(|e| e.quadrant);
        self.render_pane(frame, top_cols[0], Quadrant::Upcoming, focused_q, theme);
        self.render_pane(frame, top_cols[1], Quadrant::Overdue, focused_q, theme);
        self.render_pane(frame, bot_cols[0], Quadrant::Recent, focused_q, theme);
        self.render_pane(frame, bot_cols[1], Quadrant::Pinned, focused_q, theme);

        if let Some(confirm_area) = confirm_area {
            self.render_confirm_bar(frame, confirm_area, theme);
        }
    }

    fn captures_input(&self) -> bool {
        self.confirm_delete.is_some()
    }
}

impl Dashboard {
    fn handle_confirm_delete_key(&mut self, code: KeyCode) -> Option<AppMessage> {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(idx) = self.confirm_delete.take() {
                    if let Some(entry) = self.flat.get(idx).cloned() {
                        let item_id = entry.item.id.clone();
                        let item_kind = entry.item.kind.clone();
                        match item_kind {
                            EntityKind::Task => {
                                if let Err(e) = self.store.delete_task(&item_id) {
                                    return Some(AppMessage::Error(format!("Failed to delete task: {e}")));
                                }
                            }
                            EntityKind::Note => {
                                if let Err(e) = self.store.delete_note(&item_id) {
                                    return Some(AppMessage::Error(format!("Failed to delete note: {e}")));
                                }
                            }
                            _ => return None,
                        }
                        self.reload();
                        if self.cursor > 0 && self.cursor >= self.flat.len() {
                            self.cursor = self.flat.len().saturating_sub(1);
                        }
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

    fn render_confirm_bar(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if let Some(idx) = self.confirm_delete {
            if let Some(entry) = self.flat.get(idx) {
                let title = truncate(&entry.item.title, 30);
                let spans = vec![
                    Span::styled(format!("Delete \"{}\"? ", title), theme.warning),
                    Span::styled("(y/n)", theme.dim),
                ];
                let line = Line::from(spans);
                frame.render_widget(Paragraph::new(line), area);
            }
        }
    }

    fn render_pane(
        &self,
        frame: &mut Frame,
        area: Rect,
        quadrant: Quadrant,
        focused_q: Option<Quadrant>,
        theme: &Theme,
    ) {
        let is_focused = focused_q == Some(quadrant);

        let title_style = if is_focused {
            theme.title
        } else {
            theme.dim
        };

        let border_style = if is_focused {
            theme.accent
        } else {
            theme.border
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Span::styled(quadrant.title(), title_style));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let items = self.items_for(quadrant);
        if items.is_empty() {
            let empty_msg = match quadrant {
                Quadrant::Upcoming => "No upcoming items",
                Quadrant::Overdue => "Nothing overdue",
                Quadrant::Recent => "No recent activity",
                Quadrant::Pinned => "No pinned items",
            };
            let p = Paragraph::new(Span::styled(empty_msg, theme.dim));
            frame.render_widget(p, inner);
            return;
        }

        let max_lines = inner.height as usize;
        let display_count = items.len().min(max_lines);
        let overflow = items.len().saturating_sub(max_lines);

        let mut lines: Vec<Line> = Vec::with_capacity(display_count + if overflow > 0 { 1 } else { 0 });

        // Find global flat indices for items in this quadrant to detect cursor
        let quad_start = self.flat_index_of_quadrant_start(quadrant);

        for (i, entry) in items.iter().enumerate().take(if overflow > 0 { max_lines.saturating_sub(1) } else { max_lines }) {
            let item = &entry.item;
            let global_idx = quad_start.map(|s| {
                // Count how many items before this one in the same quadrant
                let mut idx = s;
                let mut count = 0;
                while count < i && idx < self.flat.len() {
                    if self.flat[idx].quadrant == quadrant {
                        count += 1;
                    }
                    idx += 1;
                }
                idx
            });

            let is_selected = global_idx.map(|gi| gi == self.cursor).unwrap_or(false);

            let title_text = if item.private && !self.revealed.contains(&item.id) {
                mask_private(&item.title, 8)
            } else {
                let max_title_w = inner.width.saturating_sub(4) as usize;
                truncate(&item.title, max_title_w)
            };

            let mut spans = Vec::new();

            // Icon
            let icon_style = theme.dim;
            spans.push(Span::styled(format!("{} ", item.icon()), icon_style));

            // Title
            let title_style = if item.private && !self.revealed.contains(&item.id) {
                theme.private
            } else if item.done {
                theme.status_done
            } else {
                theme.title.remove_modifier(Modifier::BOLD)
            };
            spans.push(Span::styled(title_text, title_style));

            // Due date/time
            if let Some(ref d) = item.due_date {
                let date_display = format_short_date(d);
                let today = chrono::Local::now().date_naive();
                let due_str = d.format("%Y-%m-%d").to_string();
                let date_style = if item.done {
                    theme.dim
                } else {
                    theme.due_date_style(Some(&due_str), today)
                };
                spans.push(Span::styled(format!(" {}", date_display), date_style));
            }
            if let Some(ref t) = item.due_time {
                spans.push(Span::styled(format!(" {}", t), theme.dim));
            }

            let mut line = Line::from(spans);
            if is_selected {
                line = line.style(theme.row_gray);
            }
            lines.push(line);
        }

        if overflow > 0 {
            lines.push(Line::from(Span::styled(
                format!("  +{} more", overflow),
                theme.dim,
            )));
        }

        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, inner);
    }
}

// ── Helpers ───────────────────────────────────────────────────────

fn is_date_overdue(due: &Option<NaiveDate>, today: NaiveDate) -> bool {
    if let Some(date) = due {
        return *date < today;
    }
    false
}

fn format_short_date(date: &NaiveDate) -> String {
    crate::util::date_format::format_date(date)
}
