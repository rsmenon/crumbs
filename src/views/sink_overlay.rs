use std::sync::Arc;

use chrono::Utc;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::message::AppMessage;
use crate::app::theme::Theme;
use crate::domain::{new_id, Note, Person, Refs, Tag, Task, TaskStatus};
use crate::parser::{parse_sink, SinkEntryType};
use crate::store::Store;
use crate::views::detect_private;

use super::View;

/// Autocomplete mode: whether we are completing a @person or #topic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutocompleteKind {
    Person,
    Topic,
}

/// State for the autocomplete dropdown.
#[derive(Debug, Clone)]
struct Autocomplete {
    kind: AutocompleteKind,
    /// The prefix being typed after @ or # (e.g. "ali" for "@ali").
    prefix: String,
    /// Filtered suggestions.
    suggestions: Vec<String>,
    /// Currently highlighted suggestion index.
    selected: usize,
    /// The byte offset in input where the trigger character (@ or #) is.
    trigger_pos: usize,
}

pub struct SinkOverlay {
    store: Arc<dyn Store>,
    /// Text input buffer.
    input: String,
    /// Cursor position (byte offset into `input`).
    cursor: usize,
    /// Flash message shown after a save.
    flash: Option<String>,
    /// When the flash message should be cleared.
    flash_clear_at: Option<std::time::Instant>,
    /// Autocomplete state, if active.
    autocomplete: Option<Autocomplete>,
    /// Cached list of known people slugs.
    known_people: Vec<String>,
    /// Cached list of known topic slugs.
    known_topics: Vec<String>,
}

impl SinkOverlay {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self {
            store,
            input: String::new(),
            cursor: 0,
            flash: None,
            flash_clear_at: None,
            autocomplete: None,
            known_people: Vec::new(),
            known_topics: Vec::new(),
        }
    }

    /// Load known people/topics from the store for autocomplete.
    fn refresh_data(&mut self) {
        self.known_people = self
            .store
            .list_persons()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.slug)
            .collect();

        self.known_topics = self
            .store
            .list_tags()
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.slug)
            .collect();
    }

    /// Insert a character at the cursor position and advance the cursor.
    fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.update_autocomplete();
    }

    /// Delete the character before the cursor (backspace).
    fn delete_char_before(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = crate::util::cursor_prev(&self.input, self.cursor);
        self.input.drain(prev..self.cursor);
        self.cursor = prev;
        self.update_autocomplete();
    }

    /// Delete the character at the cursor (delete key).
    fn delete_char_at(&mut self) {
        if self.cursor >= self.input.len() {
            return;
        }
        let next = crate::util::cursor_next(&self.input, self.cursor);
        self.input.drain(self.cursor..next);
        self.update_autocomplete();
    }

    /// Move cursor left by one character.
    fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor = crate::util::cursor_prev(&self.input, self.cursor);
    }

    /// Move cursor right by one character.
    fn move_right(&mut self) {
        if self.cursor >= self.input.len() {
            return;
        }
        self.cursor = crate::util::cursor_next(&self.input, self.cursor);
    }

    /// Check if we should be in autocomplete mode and update suggestions.
    fn update_autocomplete(&mut self) {
        let text_before = &self.input[..self.cursor];

        // Search backwards for an unfinished @ or # trigger.
        if let Some(ac) = self.find_trigger(text_before) {
            let suggestions = match ac.kind {
                AutocompleteKind::Person => self
                    .known_people
                    .iter()
                    .filter(|s| s.starts_with(&ac.prefix))
                    .cloned()
                    .collect::<Vec<_>>(),
                AutocompleteKind::Topic => self
                    .known_topics
                    .iter()
                    .filter(|s| s.starts_with(&ac.prefix))
                    .cloned()
                    .collect::<Vec<_>>(),
            };

            if suggestions.is_empty() {
                self.autocomplete = None;
            } else {
                let selected = ac.selected.min(suggestions.len().saturating_sub(1));
                self.autocomplete = Some(Autocomplete {
                    suggestions,
                    selected,
                    ..ac
                });
            }
        } else {
            self.autocomplete = None;
        }
    }

    /// Look backwards from cursor for an @ or # trigger that starts a
    /// partial mention/topic.
    fn find_trigger(&self, text_before: &str) -> Option<Autocomplete> {
        // Find the last @ or # in text_before that is preceded by whitespace
        // or is at position 0.
        let bytes = text_before.as_bytes();
        let mut i = bytes.len();
        while i > 0 {
            i -= 1;
            let ch = bytes[i];
            if ch == b'@' || ch == b'#' {
                // Check that it is preceded by whitespace or start of string.
                let valid_start = i == 0 || bytes[i - 1].is_ascii_whitespace();
                if !valid_start {
                    continue;
                }
                let kind = if ch == b'@' {
                    AutocompleteKind::Person
                } else {
                    AutocompleteKind::Topic
                };
                let prefix = text_before[i + 1..].to_lowercase();
                // If the prefix contains whitespace, this is not a valid trigger.
                if prefix.contains(char::is_whitespace) {
                    return None;
                }
                return Some(Autocomplete {
                    kind,
                    prefix,
                    suggestions: Vec::new(),
                    selected: 0,
                    trigger_pos: i,
                });
            }
            // If we hit whitespace, stop looking.
            if ch.is_ascii_whitespace() {
                return None;
            }
        }
        None
    }

    /// Accept the currently selected autocomplete suggestion.
    fn accept_autocomplete(&mut self) {
        let ac = match self.autocomplete.take() {
            Some(ac) => ac,
            None => return,
        };

        if ac.suggestions.is_empty() {
            return;
        }

        let suggestion = &ac.suggestions[ac.selected];
        // Replace from trigger_pos+1 to cursor with the full suggestion.
        let trigger_end = ac.trigger_pos + 1; // byte after @ or #
        let replacement = format!("{} ", suggestion);
        self.input.replace_range(trigger_end..self.cursor, &replacement);
        self.cursor = trigger_end + replacement.len();
        self.autocomplete = None;
    }

    /// Save the current input as either a Task (for todo: prefix) or append
    /// to the daily sink Note (everything else).
    fn save_entry(&mut self) -> Option<AppMessage> {
        let raw = self.input.trim().to_string();
        if raw.is_empty() {
            return None;
        }

        let parsed = parse_sink(&raw);
        let (_clean_text, is_private) = detect_private(&raw);
        let now = Utc::now();

        let refs = Refs {
            people: parsed.people.clone(),
            tags: parsed.tags.clone(),
            ..Default::default()
        };

        let cwd = std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default();

        let flash_msg: String;

        if parsed.entry_type == SinkEntryType::Todo {
            // ── Create a Task ────────────────────────────────────────
            let task = Task {
                id: new_id(),
                title: parsed.body.clone(),
                status: TaskStatus::Todo,
                due_date: parsed.datetime,
                private: is_private,
                created_dir: cwd,
                refs: refs.clone(),
                created_at: now,
                updated_at: now,
                ..Default::default()
            };

            if let Err(e) = self.store.save_task(&task) {
                return Some(AppMessage::Error(format!("Task save failed: {}", e)));
            }
            flash_msg = "\u{2714} Task created".to_string();
        } else {
            // ── Append to daily sink note ────────────────────────────
            let local_now = chrono::Local::now();
            let local_date = local_now.date_naive();
            let date_str = local_date.format("%Y-%m-%d").to_string();
            let note_id = format!("sink-{}", date_str);
            let note_title = format!("Sink \u{2014} {}", local_date.format("%b %d, %Y"));
            let entry_line = format!(
                "- {} \u{2014} {}",
                local_now.format("%H:%M"),
                raw
            );

            let mut note = match self.store.get_note(&note_id) {
                Ok(existing) => existing,
                Err(_) => Note {
                    id: note_id.clone(),
                    title: note_title,
                    created_at: now,
                    updated_at: now,
                    private: false,
                    pinned: false,
                    archived: false,
                    created_dir: cwd.clone(),
                    refs: Refs::default(),
                    body: String::new(),
                },
            };

            // Prepend the new entry line.
            if note.body.is_empty() {
                note.body = format!("{}\n", entry_line);
            } else {
                note.body = format!("{}\n{}", entry_line, note.body);
            }

            // Merge refs (dedup).
            for person in &parsed.people {
                if !note.refs.people.contains(person) {
                    note.refs.people.push(person.clone());
                }
            }
            for topic in &parsed.tags {
                if !note.refs.tags.contains(topic) {
                    note.refs.tags.push(topic.clone());
                }
            }

            note.updated_at = now;

            if let Err(e) = self.store.save_note(&note) {
                return Some(AppMessage::Error(format!("Note save failed: {}", e)));
            }
            flash_msg = "\u{2714} Saved".to_string();
        }

        // Auto-create Person/Topic records for @mentions/#topics.
        for slug in &parsed.people {
            if self.store.get_person(slug).is_err() {
                let person = Person {
                    slug: slug.clone(),
                    created_at: now,
                    pinned: false,
                    archived: false,
                    metadata: Default::default(),
                };
                if let Err(e) = self.store.save_person(&person) {
                    return Some(AppMessage::Error(format!("Failed to save person: {e}")));
                }
            }
        }
        for slug in &parsed.tags {
            if self.store.get_tag(slug).is_err() {
                let tag = Tag {
                    slug: slug.clone(),
                    created_at: now,
                };
                if let Err(e) = self.store.save_tag(&tag) {
                    return Some(AppMessage::Error(format!("Failed to save tag: {e}")));
                }
            }
        }

        self.flash = Some(flash_msg);
        self.flash_clear_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(1));
        self.input.clear();
        self.cursor = 0;
        self.autocomplete = None;

        // Refresh data to show the new entry in the recent list.
        self.refresh_data();

        Some(AppMessage::Reload)
    }
}

impl View for SinkOverlay {
    fn handle_event(&mut self, event: &Event) -> Option<AppMessage> {
        // Clear expired flash message
        if let Some(clear_at) = self.flash_clear_at {
            if std::time::Instant::now() >= clear_at {
                self.flash = None;
                self.flash_clear_at = None;
            }
        }

        let Event::Key(KeyEvent {
            code, modifiers, ..
        }) = event
        else {
            return None;
        };

        // If autocomplete is showing, handle navigation keys.
        if let Some(ref mut ac) = self.autocomplete {
            match code {
                KeyCode::Tab | KeyCode::Down => {
                    ac.selected = (ac.selected + 1).min(ac.suggestions.len().saturating_sub(1));
                    return None;
                }
                KeyCode::Up => {
                    ac.selected = ac.selected.saturating_sub(1);
                    return None;
                }
                KeyCode::Enter => {
                    // Accept the autocomplete suggestion instead of saving.
                    self.accept_autocomplete();
                    return None;
                }
                KeyCode::Esc => {
                    // Dismiss autocomplete, do not close the overlay.
                    self.autocomplete = None;
                    return None;
                }
                _ => {
                    // Fall through to normal input handling; autocomplete
                    // will be re-evaluated after the character is inserted.
                }
            }
        }

        match code {
            KeyCode::Esc => {
                self.flash = None;
                self.flash_clear_at = None;
                return Some(AppMessage::CloseSink);
            }
            KeyCode::Enter => {
                return self.save_entry();
            }
            KeyCode::Backspace => {
                self.delete_char_before();
            }
            KeyCode::Delete => {
                self.delete_char_at();
            }
            KeyCode::Left => {
                self.move_left();
                self.update_autocomplete();
            }
            KeyCode::Right => {
                self.move_right();
                self.update_autocomplete();
            }
            KeyCode::Home => {
                self.cursor = 0;
                self.update_autocomplete();
            }
            KeyCode::End => {
                self.cursor = self.input.len();
                self.update_autocomplete();
            }
            KeyCode::Char(c) => {
                // Ctrl+U clears the line.
                if *c == 'u' && modifiers.contains(KeyModifiers::CONTROL) {
                    self.input.clear();
                    self.cursor = 0;
                    self.autocomplete = None;
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
            AppMessage::OpenSink | AppMessage::Reload => {
                self.refresh_data();
                // Don't clear flash on Reload — let it expire naturally
                if matches!(msg, AppMessage::OpenSink) {
                    self.flash = None;
                    self.flash_clear_at = None;
                }
            }
            _ => {}
        }
    }

    fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        // Calculate overlay dimensions: centered, ~60% width, ~70% height.
        let overlay_w = (area.width as u32 * 60 / 100).max(40).min(area.width as u32) as u16;
        let overlay_h = 9u16.max(4).min(area.height); // prompt + input + autocomplete + flash + hint bar
        let x = area.x + (area.width.saturating_sub(overlay_w)) / 2;
        let y = area.y + (area.height.saturating_sub(overlay_h)) / 2;
        let overlay_rect = Rect::new(x, y, overlay_w, overlay_h);

        // Clear background.
        frame.render_widget(Clear, overlay_rect);

        let block = Block::default()
            .title(" SINK ")
            .title_style(theme.title)
            .borders(Borders::ALL)
            .border_style(theme.border)
            .style(theme.popup_bg);

        let inner = block.inner(overlay_rect);
        frame.render_widget(block, overlay_rect);

        if inner.height == 0 || inner.width < 4 {
            return;
        }

        // Layout: prompt line, input, (optional autocomplete), (optional flash), separator, recent entries.
        let mut constraints = vec![
            Constraint::Length(1), // prompt label
            Constraint::Length(1), // input line
        ];

        // Autocomplete dropdown takes up to 5 lines.
        let ac_lines = self
            .autocomplete
            .as_ref()
            .map(|ac| ac.suggestions.len().min(5) as u16)
            .unwrap_or(0);
        if ac_lines > 0 {
            constraints.push(Constraint::Length(ac_lines));
        }

        if self.flash.is_some() {
            constraints.push(Constraint::Length(1)); // flash
        }
        constraints.push(Constraint::Min(0)); // spacer
        constraints.push(Constraint::Length(1)); // hint bar

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let mut chunk_idx = 0;

        // -- Prompt label --
        let label = Line::from(vec![
            Span::styled(" > ", theme.accent),
            Span::styled(
                "todo: creates task \u{00b7} anything else \u{2192} daily note \u{00b7} @person #topic",
                theme.dim,
            ),
        ]);
        frame.render_widget(Paragraph::new(label), chunks[chunk_idx]);
        chunk_idx += 1;

        // -- Input line with cursor --
        let input_area = chunks[chunk_idx];
        chunk_idx += 1;
        self.render_input(frame, input_area, theme);

        // -- Autocomplete dropdown --
        if ac_lines > 0 {
            let ac_area = chunks[chunk_idx];
            chunk_idx += 1;
            self.render_autocomplete(frame, ac_area, theme);
        }

        // -- Flash message --
        if let Some(ref flash) = self.flash {
            let flash_line = Line::from(Span::styled(
                format!("  {}", flash),
                theme.success,
            ));
            frame.render_widget(Paragraph::new(flash_line), chunks[chunk_idx]);
            chunk_idx += 1;
        }

        // -- Hint bar (last chunk) --
        let hint_area = *chunks.last().unwrap();
        super::render_hint_bar(frame, hint_area, &[
            ("Enter", "save"),
            ("Esc", "cancel"),
            ("@", "person"),
            ("#", "topic"),
            ("[p]", "private"),
        ], theme);
        let _ = chunk_idx; // suppress unused warning
    }

    fn captures_input(&self) -> bool {
        true
    }
}

// ── Rendering helpers ────────────────────────────────────────────────

impl SinkOverlay {
    fn render_input(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if area.width < 4 {
            return;
        }

        let prefix = " > ";
        let avail = (area.width as usize).saturating_sub(prefix.len());

        // Build spans: text before cursor, cursor char (inverted), text after cursor.
        let before = &self.input[..self.cursor];
        let (cursor_ch, after) = if self.cursor < self.input.len() {
            // Find the end of the current character.
            let mut end = self.cursor + 1;
            while end < self.input.len() && !self.input.is_char_boundary(end) {
                end += 1;
            }
            (&self.input[self.cursor..end], &self.input[end..])
        } else {
            (" ", "") // cursor at end: show a space placeholder
        };

        // Compute visible window: if the input is wider than available space,
        // scroll so the cursor is visible.
        let cursor_display_pos = before.len();
        let total_display = before.len() + cursor_ch.len() + after.len();

        // Simple approach: if total fits, show everything; otherwise trim from left.
        let (vis_before, vis_cursor, vis_after) = if total_display <= avail {
            (before.to_string(), cursor_ch.to_string(), after.to_string())
        } else {
            // Ensure cursor is visible by showing up to `avail` chars around it.
            let start = cursor_display_pos.saturating_sub(avail / 2);
            let end_bound = (start + avail).min(self.input.len());
            let visible_input = &self.input[start..end_bound];
            if self.cursor >= start && self.cursor < end_bound {
                let local_cursor = self.cursor - start;
                let vb = &visible_input[..local_cursor];
                let mut ce = local_cursor + 1;
                while ce < visible_input.len() && !visible_input.is_char_boundary(ce) {
                    ce += 1;
                }
                let vc = if ce <= visible_input.len() {
                    &visible_input[local_cursor..ce]
                } else {
                    " "
                };
                let va = if ce < visible_input.len() {
                    &visible_input[ce..]
                } else {
                    ""
                };
                (vb.to_string(), vc.to_string(), va.to_string())
            } else {
                (visible_input.to_string(), " ".to_string(), String::new())
            }
        };

        let cursor_style = Style::default()
            .add_modifier(Modifier::REVERSED)
            .fg(theme.cursor);

        let line = Line::from(vec![
            Span::styled(prefix, theme.accent),
            Span::raw(vis_before),
            Span::styled(vis_cursor, cursor_style),
            Span::raw(vis_after),
        ]);
        frame.render_widget(Paragraph::new(line), area);
    }

    fn render_autocomplete(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let ac = match &self.autocomplete {
            Some(ac) => ac,
            None => return,
        };

        let trigger_char = match ac.kind {
            AutocompleteKind::Person => "@",
            AutocompleteKind::Topic => "#",
        };
        let style_highlight = match ac.kind {
            AutocompleteKind::Person => theme.person,
            AutocompleteKind::Topic => theme.topic,
        };

        let lines: Vec<Line<'_>> = ac
            .suggestions
            .iter()
            .take(area.height as usize)
            .enumerate()
            .map(|(i, s)| {
                let label = format!("   {}{}", trigger_char, s);
                if i == ac.selected {
                    Line::from(Span::styled(label, theme.selected))
                } else {
                    Line::from(Span::styled(label, style_highlight))
                }
            })
            .collect();

        frame.render_widget(Paragraph::new(lines), area);
    }

}

