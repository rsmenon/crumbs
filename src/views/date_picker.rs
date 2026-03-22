use chrono::{Datelike, Local, NaiveDate};
use crossterm::event::{Event, KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::message::AppMessage;
use crate::app::theme::Theme;
use crate::util::calendar::{month_grid, weekday_headers};

use super::{render_hint_bar, View};

pub struct DatePickerOverlay {
    year: i32,
    month: u32,
    grid: [[u8; 7]; 6],
    row: usize,
    col: usize,
    selected_date: NaiveDate,
}

impl DatePickerOverlay {
    pub fn new() -> Self {
        let today = Local::now().date_naive();
        let mut s = Self {
            year: today.year(),
            month: today.month(),
            grid: month_grid(today.year(), today.month()),
            row: 0,
            col: 0,
            selected_date: today,
        };
        if let Some((r, c)) = find_day(&s.grid, today.day() as u8) {
            s.row = r;
            s.col = c;
        }
        s
    }

    /// Position the picker on `date` (defaults to today).
    pub fn open(&mut self, date: Option<NaiveDate>) {
        let target = date.unwrap_or_else(|| Local::now().date_naive());
        self.year = target.year();
        self.month = target.month();
        self.grid = month_grid(self.year, self.month);
        if let Some((r, c)) = find_day(&self.grid, target.day() as u8) {
            self.row = r;
            self.col = c;
        } else {
            self.snap_to_valid();
        }
        self.sync_selected();
    }

    fn sync_selected(&mut self) {
        let day = self.grid[self.row][self.col];
        if day == 0 {
            return;
        }
        if let Some(d) = NaiveDate::from_ymd_opt(self.year, self.month, day as u32) {
            self.selected_date = d;
        }
    }

    fn snap_to_valid(&mut self) {
        if self.grid[self.row][self.col] != 0 {
            self.sync_selected();
            return;
        }
        for r in 0..6 {
            for c in 0..7 {
                if self.grid[r][c] != 0 {
                    self.row = r;
                    self.col = c;
                    self.sync_selected();
                    return;
                }
            }
        }
    }

    fn move_up(&mut self) {
        if self.row == 0 {
            return;
        }
        let mut r = self.row - 1;
        loop {
            if self.grid[r][self.col] != 0 {
                self.row = r;
                self.sync_selected();
                return;
            }
            if r == 0 {
                break;
            }
            r -= 1;
        }
    }

    fn move_down(&mut self) {
        let mut r = self.row + 1;
        while r < 6 {
            if self.grid[r][self.col] != 0 {
                self.row = r;
                self.sync_selected();
                return;
            }
            r += 1;
        }
    }

    fn move_left(&mut self) {
        if self.col == 0 {
            return;
        }
        let mut c = self.col - 1;
        loop {
            if self.grid[self.row][c] != 0 {
                self.col = c;
                self.sync_selected();
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
                self.sync_selected();
                return;
            }
            c += 1;
        }
        // Wrap to next row
        if self.row < 5 && self.grid[self.row + 1][0] != 0 {
            self.row += 1;
            self.col = 0;
            self.sync_selected();
        }
    }

    fn go_prev_month(&mut self) {
        if self.month == 1 {
            self.month = 12;
            self.year -= 1;
        } else {
            self.month -= 1;
        }
        self.grid = month_grid(self.year, self.month);
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
        self.grid = month_grid(self.year, self.month);
        self.row = 0;
        self.col = 0;
        self.snap_to_valid();
    }

    fn go_today(&mut self) {
        let today = Local::now().date_naive();
        self.year = today.year();
        self.month = today.month();
        self.grid = month_grid(self.year, self.month);
        if let Some((r, c)) = find_day(&self.grid, today.day() as u8) {
            self.row = r;
            self.col = c;
        }
        self.sync_selected();
    }
}

impl View for DatePickerOverlay {
    fn handle_event(&mut self, event: &Event) -> Option<AppMessage> {
        let Event::Key(KeyEvent { code, .. }) = event else {
            return None;
        };
        match code {
            KeyCode::Char('h') | KeyCode::Left  => { self.move_left();      None }
            KeyCode::Char('l') | KeyCode::Right => { self.move_right();     None }
            KeyCode::Char('k') | KeyCode::Up    => { self.move_up();        None }
            KeyCode::Char('j') | KeyCode::Down  => { self.move_down();      None }
            KeyCode::Char('[')                  => { self.go_prev_month();  None }
            KeyCode::Char(']')                  => { self.go_next_month();  None }
            KeyCode::Char('t')                  => { self.go_today();       None }
            KeyCode::Enter => Some(AppMessage::DatePickerConfirm(self.selected_date)),
            KeyCode::Esc   => Some(AppMessage::DatePickerCancel),
            _ => None,
        }
    }

    fn handle_message(&mut self, _msg: &AppMessage) {}

    fn captures_input(&self) -> bool {
        true
    }

    fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        // Each cell is 5 chars wide (` XX  `), 7 cols = 35, plus 2 border = 37.
        // Add 2 padding = 39.
        let popup_w: u16 = 39;
        // 1 (headers) + 6 (grid) + 1 (hint) = 8 inner + 2 border = 10
        let popup_h: u16 = 10;

        let x = area.x + area.width.saturating_sub(popup_w) / 2;
        let y = area.y + area.height.saturating_sub(popup_h) / 2;
        let popup_rect = Rect::new(x, y, popup_w.min(area.width), popup_h.min(area.height));

        frame.render_widget(Clear, popup_rect);

        let month_label = MONTH_NAMES[(self.month - 1) as usize];
        let title = format!(" {} {} ", month_label, self.year);
        let block = Block::default()
            .title(title)
            .title_style(theme.title)
            .borders(Borders::ALL)
            .border_style(theme.border)
            .style(theme.popup_bg);

        let inner = block.inner(popup_rect);
        frame.render_widget(block, popup_rect);

        if inner.height < 2 || inner.width < 35 {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // weekday headers
                Constraint::Min(6),    // grid rows
                Constraint::Length(1), // hint bar
            ])
            .split(inner);

        // Weekday headers
        let headers = weekday_headers();
        let header_spans: Vec<Span> = headers
            .iter()
            .map(|h| Span::styled(format!(" {:>2}  ", h), theme.column_header))
            .collect();
        frame.render_widget(Paragraph::new(Line::from(header_spans)), chunks[0]);

        // Grid
        let today = Local::now().date_naive();
        let available_rows = (chunks[1].height as usize).min(6);
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

                let cell = format!(" {:>2}  ", day);
                let style = if is_cursor {
                    theme.selected
                } else if is_today {
                    theme.accent.add_modifier(Modifier::BOLD)
                } else {
                    theme.dim
                };
                spans.push(Span::styled(cell, style));
            }
            lines.push(Line::from(spans));
        }

        frame.render_widget(Paragraph::new(lines), chunks[1]);

        // Hint bar
        render_hint_bar(
            frame,
            chunks[2],
            &[
                ("hjkl", "navigate"),
                ("[/]", "month"),
                ("t", "today"),
                ("↵", "confirm"),
                ("Esc", "cancel"),
            ],
            theme,
        );
    }
}

fn find_day(grid: &[[u8; 7]; 6], day: u8) -> Option<(usize, usize)> {
    for r in 0..6 {
        for c in 0..7 {
            if grid[r][c] == day {
                return Some((r, c));
            }
        }
    }
    None
}

const MONTH_NAMES: [&str; 12] = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
];
