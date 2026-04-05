use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::message::AppMessage;
use crate::app::theme::Theme;
use crate::domain::{EntityKind, EntityRef};
use crate::store::Store;
use crate::util::date_format;

use super::icons;
use super::View;

/// A resolved entity for display in the link overlay.
#[derive(Debug, Clone)]
struct LinkResult {
    entity_ref: EntityRef,
    already_linked: bool,
    icon: &'static str,
    title: String,
    /// Secondary info: status + tags + date (varies by entity kind).
    detail: String,
    /// Style for the detail text (e.g. status color for tasks).
    detail_style_key: DetailStyle,
}

#[derive(Debug, Clone, Copy)]
enum DetailStyle {
    Status(StatusColor),
    Dim,
}

#[derive(Debug, Clone, Copy)]
pub enum StatusColor {
    Backlog,
    Todo,
    InProgress,
    Blocked,
    Done,
    Archived,
}

pub struct LinkOverlay {
    store: Arc<dyn Store>,
    input: String,
    cursor: usize,
    results: Vec<LinkResult>,
    selected: usize,
    /// The entity being linked from.
    source_kind: EntityKind,
    source_id: String,
    /// IDs of entities already linked to the source (for "already linked" indicator).
    linked_ids: std::collections::HashSet<String>,
    active: bool,
}

impl LinkOverlay {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self {
            store,
            input: String::new(),
            cursor: 0,
            results: Vec::new(),
            selected: 0,
            source_kind: EntityKind::Task,
            source_id: String::new(),
            linked_ids: std::collections::HashSet::new(),
            active: false,
        }
    }

    /// Open the overlay for a given source entity.
    pub fn open(&mut self, source_kind: EntityKind, source_id: String) {
        self.source_kind = source_kind.clone();
        self.source_id = source_id.clone();
        self.input.clear();
        self.cursor = 0;
        self.results.clear();
        self.selected = 0;
        self.active = true;

        // Load current linked IDs from the source entity's refs.
        self.linked_ids.clear();
        let refs = match &source_kind {
            EntityKind::Task => {
                self.store.get_task(&source_id).ok().map(|t| t.refs)
            }
            EntityKind::Note => {
                self.store.get_note(&source_id).ok().map(|n| n.refs)
            }
            EntityKind::Agenda => {
                self.store.get_agenda(&source_id).ok().map(|a| a.refs)
            }
            _ => None,
        };
        if let Some(refs) = refs {
            for id in refs.tasks { self.linked_ids.insert(id); }
            for id in refs.notes { self.linked_ids.insert(id); }
            for id in refs.agendas { self.linked_ids.insert(id); }
        }
    }

    fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.run_search();
    }

    fn delete_char_before(&mut self) {
        if self.cursor == 0 { return; }
        let prev = crate::util::cursor_prev(&self.input, self.cursor);
        self.input.drain(prev..self.cursor);
        self.cursor = prev;
        self.run_search();
    }

    fn delete_char_at(&mut self) {
        if self.cursor >= self.input.len() { return; }
        let next = crate::util::cursor_next(&self.input, self.cursor);
        self.input.drain(self.cursor..next);
        self.run_search();
    }

    fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = crate::util::cursor_prev(&self.input, self.cursor);
        }
    }

    fn move_right(&mut self) {
        if self.cursor < self.input.len() {
            self.cursor = crate::util::cursor_next(&self.input, self.cursor);
        }
    }

    fn run_search(&mut self) {
        let query = self.input.trim().to_string();
        if query.is_empty() {
            self.results.clear();
            self.selected = 0;
            return;
        }

        let source_id = self.source_id.clone();
        let source_kind = self.source_kind.clone();

        let refs = self.store.search(&query);
        self.results = refs
            .into_iter()
            // Only link to tasks, notes, and agendas (not people/tags).
            .filter(|eref| matches!(eref.kind, EntityKind::Task | EntityKind::Note | EntityKind::Agenda))
            // Exclude self.
            .filter(|eref| !(eref.kind == source_kind && eref.id == source_id))
            .take(50)
            .filter_map(|eref| self.resolve_ref(eref))
            .collect();

        if self.selected >= self.results.len() {
            self.selected = self.results.len().saturating_sub(1);
        }
    }

    fn resolve_ref(&self, eref: EntityRef) -> Option<LinkResult> {
        let id = eref.id.clone();
        let already_linked = self.linked_ids.contains(&id);

        match &eref.kind {
            EntityKind::Task => {
                let task = self.store.get_task(&id).ok()?;
                let title = if task.private { "********".to_string() } else { task.title.clone() };
                let tags: String = task.refs.tags.iter()
                    .map(|t| format!("#{}", t))
                    .collect::<Vec<_>>()
                    .join(" ");
                let due = task.due_date
                    .map(|d| format!(" Due:{}", date_format::format_date(&d)))
                    .unwrap_or_default();
                let detail = format!("{} {}{}{}",
                    task.status.icon(),
                    task.status.label(),
                    if tags.is_empty() { String::new() } else { format!("  {}", tags) },
                    due,
                );
                let status_color = match task.status {
                    crate::domain::TaskStatus::Backlog   => StatusColor::Backlog,
                    crate::domain::TaskStatus::Todo      => StatusColor::Todo,
                    crate::domain::TaskStatus::InProgress => StatusColor::InProgress,
                    crate::domain::TaskStatus::Blocked   => StatusColor::Blocked,
                    crate::domain::TaskStatus::Done      => StatusColor::Done,
                    crate::domain::TaskStatus::Archived  => StatusColor::Archived,
                };
                Some(LinkResult {
                    entity_ref: eref,
                    already_linked,
                    icon: icons::TASK,
                    title,
                    detail,
                    detail_style_key: DetailStyle::Status(status_color),
                })
            }
            EntityKind::Note => {
                let note = self.store.get_note(&id).ok()?;
                let title = if note.private { "********".to_string() } else { note.title.clone() };
                let tags: String = note.refs.tags.iter()
                    .map(|t| format!("#{}", t))
                    .collect::<Vec<_>>()
                    .join(" ");
                let created = date_format::format_utc_date(&note.created_at);
                let detail = format!("{}{}  Created:{}",
                    if tags.is_empty() { String::new() } else { format!("{}  ", tags) },
                    String::new(),
                    created,
                );
                Some(LinkResult {
                    entity_ref: eref,
                    already_linked,
                    icon: icons::NOTE,
                    title,
                    detail,
                    detail_style_key: DetailStyle::Dim,
                })
            }
            EntityKind::Agenda => {
                let agenda = self.store.get_agenda(&id).ok()?;
                let detail = format!("@{}  {}", agenda.person_slug, agenda.date.format("%Y-%m-%d"));
                Some(LinkResult {
                    entity_ref: eref,
                    already_linked,
                    icon: icons::AGENDA,
                    title: agenda.title.clone(),
                    detail,
                    detail_style_key: DetailStyle::Dim,
                })
            }
            _ => None,
        }
    }

    /// Perform the link action for the currently selected result.
    fn do_link(&mut self) -> Option<AppMessage> {
        let result = self.results.get(self.selected)?;
        let tgt_kind = match result.entity_ref.kind {
            EntityKind::Task => "task",
            EntityKind::Note => "note",
            EntityKind::Agenda => "agenda",
            _ => return None,
        };
        let src_kind = match self.source_kind {
            EntityKind::Task => "task",
            EntityKind::Note => "note",
            EntityKind::Agenda => "agenda",
            _ => return None,
        };

        if let Err(e) = self.store.add_entity_ref(src_kind, &self.source_id, tgt_kind, &result.entity_ref.id) {
            return Some(AppMessage::Error(format!("Failed to link: {e}")));
        }

        // Mark as linked in local state so the indicator updates immediately.
        self.linked_ids.insert(result.entity_ref.id.clone());

        // Re-run search to refresh "already linked" state in results.
        let q = self.input.clone();
        if !q.is_empty() {
            self.run_search();
        }

        Some(AppMessage::Reload)
    }
}

impl View for LinkOverlay {
    fn handle_event(&mut self, event: &Event) -> Option<AppMessage> {
        let Event::Key(KeyEvent { code, modifiers, .. }) = event else {
            return None;
        };

        match code {
            KeyCode::Esc => {
                self.active = false;
                return Some(AppMessage::CloseLinkOverlay);
            }
            KeyCode::Enter => {
                return self.do_link();
            }
            KeyCode::Down => {
                if !self.results.is_empty() {
                    self.selected = (self.selected + 1).min(self.results.len().saturating_sub(1));
                }
                return None;
            }
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                return None;
            }
            KeyCode::Backspace => { self.delete_char_before(); }
            KeyCode::Delete => { self.delete_char_at(); }
            KeyCode::Left => { self.move_left(); }
            KeyCode::Right => { self.move_right(); }
            KeyCode::Home => { self.cursor = 0; }
            KeyCode::End => { self.cursor = self.input.len(); }
            KeyCode::Char(c) => {
                if *c == 'u' && modifiers.contains(KeyModifiers::CONTROL) {
                    self.input.clear();
                    self.cursor = 0;
                    self.results.clear();
                    self.selected = 0;
                } else {
                    self.insert_char(*c);
                }
            }
            _ => {}
        }

        None
    }

    fn handle_message(&mut self, _msg: &AppMessage) {}

    fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let overlay_w = (area.width as u32 * 65 / 100).max(50).min(area.width as u32) as u16;
        let overlay_h = (area.height as u32 * 65 / 100).max(10).min(area.height as u32) as u16;
        let x = area.x + (area.width.saturating_sub(overlay_w)) / 2;
        let y = area.y + (area.height.saturating_sub(overlay_h)) / 2;
        let overlay_rect = Rect::new(x, y, overlay_w, overlay_h);

        frame.render_widget(Clear, overlay_rect);

        let block = Block::default()
            .title(" Link to... ")
            .title_style(theme.title)
            .borders(Borders::ALL)
            .border_style(theme.border)
            .style(theme.popup_bg);

        let inner = block.inner(overlay_rect);
        frame.render_widget(block, overlay_rect);

        if inner.height == 0 || inner.width < 4 {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // input
                Constraint::Length(1), // separator
                Constraint::Min(1),    // results
                Constraint::Length(1), // hint bar
            ])
            .split(inner);

        self.render_input(frame, chunks[0], theme);

        let sep = "─".repeat(chunks[1].width as usize);
        frame.render_widget(Paragraph::new(Span::styled(sep, theme.border)), chunks[1]);

        self.render_results(frame, chunks[2], theme);

        super::render_hint_bar(frame, chunks[3], &[
            ("↑↓", "navigate results"),
            ("Enter", "link"),
            ("Esc", "close"),
        ], theme);
    }

    fn captures_input(&self) -> bool {
        true
    }
}

impl LinkOverlay {
    fn render_input(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let prefix = " \u{f0337} "; // 󰌷 nf-md-link
        let before = &self.input[..self.cursor];
        let (cursor_ch, after) = if self.cursor < self.input.len() {
            let mut end = self.cursor + 1;
            while end < self.input.len() && !self.input.is_char_boundary(end) {
                end += 1;
            }
            (&self.input[self.cursor..end], &self.input[end..])
        } else {
            (" ", "")
        };

        let cursor_style = Style::default().add_modifier(Modifier::REVERSED).fg(theme.cursor);
        let line = Line::from(vec![
            Span::styled(prefix, theme.accent),
            Span::raw(before),
            Span::styled(cursor_ch, cursor_style),
            Span::raw(after),
        ]);
        frame.render_widget(Paragraph::new(line), area);
    }

    fn render_results(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if self.results.is_empty() {
            let msg = if self.input.trim().is_empty() {
                "Type to search by title..."
            } else {
                "No results found."
            };
            frame.render_widget(
                Paragraph::new(Span::styled(format!("   {}", msg), theme.dim)),
                area,
            );
            return;
        }

        let max_visible = area.height as usize;
        let scroll_offset = if self.selected >= max_visible {
            self.selected - max_visible + 1
        } else {
            0
        };

        let lines: Vec<Line<'_>> = self.results
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(max_visible)
            .map(|(i, result)| {
                let is_selected = i == self.selected;
                let base_style = if is_selected { theme.selected } else { Style::default() };

                // Linked indicator
                let linked_indicator = if result.already_linked {
                    Span::styled(" ✓ ", theme.success)
                } else if is_selected {
                    Span::styled(" ▸ ", base_style)
                } else {
                    Span::styled("   ", base_style)
                };

                // Title width: total - icon(2) - linked(3) - separator(2) - detail(min 20)
                let detail_w = result.detail.chars().count().min(35);
                let title_w = (area.width as usize)
                    .saturating_sub(2 + 3 + 2 + detail_w + 2);
                let display_title = super::truncate(&result.title, title_w);

                // Detail style
                let detail_style = match result.detail_style_key {
                    DetailStyle::Dim => theme.dim,
                    DetailStyle::Status(sc) => {
                        use crate::domain::TaskStatus;
                        let status = match sc {
                            StatusColor::Backlog   => TaskStatus::Backlog,
                            StatusColor::Todo      => TaskStatus::Todo,
                            StatusColor::InProgress => TaskStatus::InProgress,
                            StatusColor::Blocked   => TaskStatus::Blocked,
                            StatusColor::Done      => TaskStatus::Done,
                            StatusColor::Archived  => TaskStatus::Archived,
                        };
                        theme.status_fg(&status)
                    }
                };

                Line::from(vec![
                    linked_indicator,
                    Span::styled(format!("{} ", result.icon), theme.dim),
                    Span::styled(display_title, base_style),
                    Span::styled("  ", theme.dim),
                    Span::styled(result.detail.clone(), if is_selected { base_style } else { detail_style }),
                ])
            })
            .collect();

        frame.render_widget(Paragraph::new(lines), area);
    }
}
