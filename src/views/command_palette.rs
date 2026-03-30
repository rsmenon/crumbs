use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::message::AppMessage;
use crate::app::theme::Theme;
use super::View;

/// A single command palette action with an id (emitted via PaletteAction)
/// and a human-readable label for display and fuzzy filtering.
#[derive(Debug, Clone)]
struct PaletteEntry {
    id: &'static str,
    label: &'static str,
}

/// All predefined palette actions.
static ACTIONS: &[PaletteEntry] = &[
    PaletteEntry {
        id: "dashboard",
        label: "Dashboard",
    },
    PaletteEntry {
        id: "tasks",
        label: "Tasks",
    },
    PaletteEntry {
        id: "calendar",
        label: "Calendar",
    },
    PaletteEntry {
        id: "notes",
        label: "Notes",
    },
    PaletteEntry {
        id: "people",
        label: "People",
    },
    PaletteEntry {
        id: "search",
        label: "Search",
    },
    PaletteEntry {
        id: "sink",
        label: "Sink",
    },
    PaletteEntry {
        id: "filter-tag",
        label: "Filter by tag",
    },
    PaletteEntry {
        id: "quit",
        label: "Quit",
    },
];

pub struct CommandPalette {
    /// Filter input buffer.
    input: String,
    /// Cursor position (byte offset).
    cursor: usize,
    /// Indices into ACTIONS that match the current filter.
    filtered: Vec<usize>,
    /// Currently highlighted result in the filtered list.
    selected: usize,
}

impl CommandPalette {
    pub fn new() -> Self {
        let mut cp = Self {
            input: String::new(),
            cursor: 0,
            filtered: Vec::new(),
            selected: 0,
        };
        cp.update_filter();
        cp
    }

    /// Reset state when the palette is opened.
    fn reset(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.selected = 0;
        self.update_filter();
    }

    /// Insert a character at cursor.
    fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.update_filter();
    }

    /// Delete character before cursor.
    fn delete_char_before(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut prev = self.cursor - 1;
        while prev > 0 && !self.input.is_char_boundary(prev) {
            prev -= 1;
        }
        self.input.drain(prev..self.cursor);
        self.cursor = prev;
        self.update_filter();
    }

    /// Delete character at cursor.
    fn delete_char_at(&mut self) {
        if self.cursor >= self.input.len() {
            return;
        }
        let mut next = self.cursor + 1;
        while next < self.input.len() && !self.input.is_char_boundary(next) {
            next += 1;
        }
        self.input.drain(self.cursor..next);
        self.update_filter();
    }

    /// Move cursor left.
    fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut prev = self.cursor - 1;
        while prev > 0 && !self.input.is_char_boundary(prev) {
            prev -= 1;
        }
        self.cursor = prev;
    }

    /// Move cursor right.
    fn move_right(&mut self) {
        if self.cursor >= self.input.len() {
            return;
        }
        let mut next = self.cursor + 1;
        while next < self.input.len() && !self.input.is_char_boundary(next) {
            next += 1;
        }
        self.cursor = next;
    }

    /// Update the filtered list based on the current input.
    fn update_filter(&mut self) {
        let query = self.input.to_lowercase();
        self.filtered = ACTIONS
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                if query.is_empty() {
                    return true;
                }
                entry.label.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect();

        // Clamp selection.
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
    }
}

impl View for CommandPalette {
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
                return Some(AppMessage::ClosePalette);
            }
            KeyCode::Enter => {
                if let Some(&action_idx) = self.filtered.get(self.selected) {
                    let action = &ACTIONS[action_idx];
                    let msg = AppMessage::PaletteAction(action.id.to_string());
                    self.reset();
                    return Some(msg);
                }
            }
            KeyCode::Down => {
                if !self.filtered.is_empty() {
                    self.selected =
                        (self.selected + 1).min(self.filtered.len().saturating_sub(1));
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
                    self.update_filter();
                } else {
                    self.insert_char(*c);
                }
            }
            _ => {}
        }

        None
    }

    fn handle_message(&mut self, _msg: &AppMessage) {
    }

    fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        // Overlay dimensions: centered, narrower and shorter than other overlays.
        let overlay_w = (area.width as u32 * 50 / 100).max(36).min(area.width as u32) as u16;
        // Height: input(1) + separator(1) + action rows + hint(1) + border(2).
        let content_rows = self.filtered.len().max(1) as u16;
        let overlay_h = (content_rows + 5).min(area.height);
        let x = area.x + (area.width.saturating_sub(overlay_w)) / 2;
        let y = area.y + (area.height.saturating_sub(overlay_h)) / 3; // Slightly above center.
        let overlay_rect = Rect::new(x, y, overlay_w, overlay_h);

        frame.render_widget(Clear, overlay_rect);

        let block = Block::default()
            .title(" Command Palette ")
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
                Constraint::Min(1),    // actions list
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

        // -- Filtered actions --
        self.render_actions(frame, chunks[2], theme);

        // -- Hint bar --
        super::render_hint_bar(frame, chunks[3], &[
            ("↑↓", "navigate"),
            ("Enter", "run"),
            ("Esc", "close"),
        ], theme);
    }

    fn captures_input(&self) -> bool {
        true
    }
}

// ── Rendering helpers ────────────────────────────────────────────────

impl CommandPalette {
    fn render_input(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let prefix = " > ";
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

    fn render_actions(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if self.filtered.is_empty() {
            let line = Line::from(Span::styled("   No matches", theme.dim));
            frame.render_widget(Paragraph::new(line), area);
            return;
        }

        let max_visible = area.height as usize;
        let scroll_offset = if self.selected >= max_visible {
            self.selected - max_visible + 1
        } else {
            0
        };

        let lines: Vec<Line<'_>> = self
            .filtered
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(max_visible)
            .map(|(i, &action_idx)| {
                let entry = &ACTIONS[action_idx];
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
                    Span::styled(entry.label, base_style),
                ])
            })
            .collect();

        frame.render_widget(Paragraph::new(lines), area);
    }
}
