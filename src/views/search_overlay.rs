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

use super::icons;
use super::View;

/// A resolved search result for display.
#[derive(Debug, Clone)]
struct SearchResult {
    entity_ref: EntityRef,
    icon: &'static str,
    title: String,
    date: String,
}

pub struct SearchOverlay {
    store: Arc<dyn Store>,
    /// Search query input buffer.
    input: String,
    /// Cursor position (byte offset).
    cursor: usize,
    /// Current search results.
    results: Vec<SearchResult>,
    /// Selected result index.
    selected: usize,
    /// Global tag filter.
    tag_filter: Option<String>,
}

impl SearchOverlay {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self {
            store,
            input: String::new(),
            cursor: 0,
            results: Vec::new(),
            selected: 0,
            tag_filter: None,
        }
    }

    /// Reset state when the overlay is opened.
    fn reset(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.results.clear();
        self.selected = 0;
    }

    /// Insert a character at cursor and advance.
    fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.run_search();
    }

    /// Delete the character before the cursor.
    fn delete_char_before(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = crate::util::cursor_prev(&self.input, self.cursor);
        self.input.drain(prev..self.cursor);
        self.cursor = prev;
        self.run_search();
    }

    /// Delete the character at the cursor.
    fn delete_char_at(&mut self) {
        if self.cursor >= self.input.len() {
            return;
        }
        let next = crate::util::cursor_next(&self.input, self.cursor);
        self.input.drain(self.cursor..next);
        self.run_search();
    }

    /// Move cursor left.
    fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor = crate::util::cursor_prev(&self.input, self.cursor);
    }

    /// Move cursor right.
    fn move_right(&mut self) {
        if self.cursor >= self.input.len() {
            return;
        }
        self.cursor = crate::util::cursor_next(&self.input, self.cursor);
    }

    /// Execute a search using the store and resolve entity refs to display data.
    fn run_search(&mut self) {
        let query = self.input.trim();
        if query.is_empty() {
            self.results.clear();
            self.selected = 0;
            return;
        }

        let refs = self.store.search(query);
        let tag_filter = self.tag_filter.clone();
        self.results = refs
            .into_iter()
            .filter(|eref| {
                if let Some(ref tag) = tag_filter {
                    self.entity_has_tag(eref, tag)
                } else {
                    true
                }
            })
            .take(50)
            .filter_map(|eref| self.resolve_ref(&eref))
            .collect();

        // Clamp selection.
        if self.selected >= self.results.len() {
            self.selected = self.results.len().saturating_sub(1);
        }
    }

    /// Check if an entity ref matches a tag filter.
    fn entity_has_tag(&self, eref: &EntityRef, tag: &str) -> bool {
        let id = eref.id.clone();
        match eref.kind {
            EntityKind::Task => {
                self.store.get_task(&id)
                    .map(|t| t.refs.tags.iter().any(|tg| tg == tag))
                    .unwrap_or(false)
            }
            EntityKind::Note => {
                self.store.get_note(&id)
                    .map(|n| n.refs.tags.iter().any(|tg| tg == tag))
                    .unwrap_or(false)
            }
            EntityKind::Person => {
                // People don't have a direct tags field; skip tag filtering for them.
                false
            }
            EntityKind::Tag => {
                // Tags match if their slug is the filter tag
                id == tag
            }
            _ => false,
        }
    }

    /// Resolve an EntityRef into a displayable SearchResult.
    fn resolve_ref(&self, eref: &EntityRef) -> Option<SearchResult> {
        let id = eref.id.clone();
        let fmt_utc = |dt: &chrono::DateTime<chrono::Utc>| -> String {
            crate::util::date_format::format_utc_date(dt)
        };
        let fmt_date = |d: &chrono::NaiveDate| -> String {
            crate::util::date_format::format_date(d)
        };

        let (icon, title, date) = match eref.kind {
            EntityKind::Task => {
                let task = self.store.get_task(&id).ok()?;
                let date = task
                    .due_date
                    .as_ref()
                    .map(|d| fmt_date(d))
                    .unwrap_or_else(|| fmt_utc(&task.created_at));
                let title = if task.private {
                    "********".to_string()
                } else {
                    task.title
                };
                (icons::TASK, title, date)
            }
            EntityKind::Note => {
                let note = self.store.get_note(&id).ok()?;
                let date = fmt_utc(&note.created_at);
                let title = if note.private {
                    "********".to_string()
                } else {
                    note.title
                };
                (icons::NOTE, title, date)
            }
            EntityKind::Person => {
                let p = self.store.get_person(&id).ok()?;
                let date = fmt_utc(&p.created_at);
                (icons::MEMORY, format!("@{}", p.slug), date)
            }
            EntityKind::Tag => {
                let t = self.store.get_tag(&id).ok()?;
                let date = fmt_utc(&t.created_at);
                (icons::TAG, format!("#{}", t.slug), date)
            }
            EntityKind::Agenda => {
                let a = self.store.get_agenda(&id).ok()?;
                let date = fmt_date(&a.date);
                (icons::AGENDA, a.title, date)
            }
        };

        Some(SearchResult {
            entity_ref: eref.clone(),
            icon,
            title,
            date,
        })
    }
}

impl View for SearchOverlay {
    fn handle_event(&mut self, event: &Event) -> Option<AppMessage> {
        let Event::Key(KeyEvent {
            code, modifiers, ..
        }) = event
        else {
            return None;
        };

        match code {
            KeyCode::Esc => {
                self.reset();
                return Some(AppMessage::CloseSearch);
            }
            KeyCode::Enter => {
                // Open the selected result.
                if let Some(result) = self.results.get(self.selected) {
                    let eref = &result.entity_ref;
                    let id = eref.id.clone();
                    let msg = match eref.kind {
                        EntityKind::Task => Some(AppMessage::OpenTaskEditor(id)),
                        EntityKind::Note => Some(AppMessage::OpenNoteEditor(id)),
                        EntityKind::Person => Some(AppMessage::NavigatePerson(id)),
                        _ => Some(AppMessage::NavigateRef(eref.clone())),
                    };
                    self.reset();
                    return msg;
                }
            }
            KeyCode::Down => {
                if !self.results.is_empty() {
                    self.selected =
                        (self.selected + 1).min(self.results.len().saturating_sub(1));
                }
                return None;
            }
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                return None;
            }
            KeyCode::Backspace => {
                self.delete_char_before();
            }
            KeyCode::Delete => {
                self.delete_char_at();
            }
            KeyCode::Left => {
                self.move_left();
            }
            KeyCode::Right => {
                self.move_right();
            }
            KeyCode::Home => {
                self.cursor = 0;
            }
            KeyCode::End => {
                self.cursor = self.input.len();
            }
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

    fn handle_message(&mut self, msg: &AppMessage) {
        match msg {
            AppMessage::OpenSearch => {
                self.reset();
            }
            AppMessage::TagFilterChanged(filter) => {
                self.tag_filter = filter.clone();
            }
            _ => {}
        }
    }

    fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        // Overlay dimensions: centered, ~60% width, ~70% height.
        let overlay_w = (area.width as u32 * 60 / 100).max(40).min(area.width as u32) as u16;
        let overlay_h = (area.height as u32 * 70 / 100).max(10).min(area.height as u32) as u16;
        let x = area.x + (area.width.saturating_sub(overlay_w)) / 2;
        let y = area.y + (area.height.saturating_sub(overlay_h)) / 2;
        let overlay_rect = Rect::new(x, y, overlay_w, overlay_h);

        frame.render_widget(Clear, overlay_rect);

        let block = Block::default()
            .title(" Search ")
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

        // -- Input line --
        self.render_input(frame, chunks[0], theme);

        // -- Separator --
        let sep = "─".repeat(chunks[1].width as usize);
        frame.render_widget(
            Paragraph::new(Span::styled(sep, theme.border)),
            chunks[1],
        );

        // -- Results --
        self.render_results(frame, chunks[2], theme);

        // -- Hint bar --
        super::render_hint_bar(frame, chunks[3], &[
            ("↑↓", "navigate"),
            ("Enter", "open"),
            ("Esc", "close"),
        ], theme);
    }

    fn captures_input(&self) -> bool {
        true
    }
}

// ── Rendering helpers ────────────────────────────────────────────────

impl SearchOverlay {
    fn render_input(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let prefix = " / ";
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

        let cursor_style = Style::default()
            .add_modifier(Modifier::REVERSED)
            .fg(theme.cursor);

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
                "Type to search..."
            } else {
                "No results found."
            };
            let line = Line::from(Span::styled(format!("   {}", msg), theme.dim));
            frame.render_widget(Paragraph::new(line), area);
            return;
        }

        let max_visible = area.height as usize;
        // Scroll window: keep selected item visible.
        let scroll_offset = if self.selected >= max_visible {
            self.selected - max_visible + 1
        } else {
            0
        };

        let lines: Vec<Line<'_>> = self
            .results
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(max_visible)
            .map(|(i, result)| {
                let title_w = (area.width as usize).saturating_sub(18);
                let display_title = super::truncate(&result.title, title_w);

                let is_selected = i == self.selected;
                let base_style = if is_selected {
                    theme.selected
                } else {
                    Style::default()
                };

                Line::from(vec![
                    Span::styled(
                        if is_selected { " ▸ " } else { "   " },
                        base_style,
                    ),
                    Span::styled(format!("{} ", result.icon), theme.dim),
                    Span::styled(display_title, base_style),
                    Span::styled(format!("  {}", result.date), theme.date),
                ])
            })
            .collect();

        frame.render_widget(Paragraph::new(lines), area);
    }
}
