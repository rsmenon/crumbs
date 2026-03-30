pub mod message;
pub mod theme;
pub mod help;

use std::sync::{mpsc, Arc};
use std::path::PathBuf;

use crossterm::event::Event;
use ratatui::backend::Backend;
use ratatui::Frame;
use ratatui::Terminal;

use crate::config::Config;
use crate::store::Store;
use message::AppMessage;
use theme::Theme;

use crate::views::View;
use crate::views::{
    dashboard::Dashboard,
    tasks_tab::TasksTab,
    calendar::CalendarView,
    note_view::NoteView,
    people_view::PeopleView,
    sink_overlay::SinkOverlay,
    search_overlay::SearchOverlay,
    command_palette::CommandPalette,
    nvim_overlay::NvimOverlay,
    date_picker::DatePickerOverlay,
};
use help::HelpOverlay;
use message::DatePickerContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    Dashboard,
    Tasks,
    Calendar,
    Notes,
    People,
}

impl ActiveTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dashboard => "[D]ashboard",
            Self::Tasks => "[T]asks",
            Self::Calendar => "[C]alendar",
            Self::Notes => "[N]otes",
            Self::People => "[P]eople",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::Tasks => "Tasks",
            Self::Calendar => "Calendar",
            Self::Notes => "Notes",
            Self::People => "People",
        }
    }

    pub fn status_hints(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Dashboard => &[
                ("s", "sink"), ("/", "search"), ("?", "help"),
            ],
            Self::Tasks => &[
                ("n", "new"), ("e", "edit"), ("d", "delete"),
                ("Space", "status"), ("f", "filter"), ("S", "sort"),
                ("A", "archived"),
            ],
            Self::Calendar => &[
                ("Enter", "day panel"), ("t", "today"), ("[/]", "month"),
            ],
            Self::Notes => &[
                ("Enter", "open"), ("v", "preview"),
            ],
            Self::People => &[
                ("n", "new"), ("e", "rename"),
                ("Tab", "focus pane"), ("Enter", "open"),
            ],
        }
    }

    pub const ALL: [ActiveTab; 5] = [
        Self::Dashboard,
        Self::Tasks,
        Self::Calendar,
        Self::Notes,
        Self::People,
    ];
}

pub struct App {
    pub active_tab: ActiveTab,
    pub width: u16,
    pub height: u16,
    pub theme: Theme,

    // Tab views
    pub dashboard: Dashboard,
    pub tasks_tab: TasksTab,
    pub calendar: CalendarView,
    pub notes: NoteView,
    pub people: PeopleView,

    // Overlay views
    pub sink: SinkOverlay,
    pub search: SearchOverlay,
    pub palette: CommandPalette,
    pub help: HelpOverlay,

    // Inline nvim editor overlay
    pub editor: Option<NvimOverlay>,
    pub show_editor: bool,

    // Overlay views
    pub date_picker: DatePickerOverlay,

    // Overlay visibility
    pub show_sink: bool,
    pub show_search: bool,
    pub show_palette: bool,
    pub show_help: bool,
    pub show_date_picker: bool,
    pub date_picker_context: Option<DatePickerContext>,

    // Shared state
    pub store: Arc<dyn Store>,
    pub data_dir: PathBuf,

    // Async message channel
    #[allow(dead_code)]
    pub msg_tx: mpsc::Sender<AppMessage>,
    pub msg_rx: mpsc::Receiver<AppMessage>,

    // Error flash
    pub error_flash: Option<String>,
    pub error_clear_at: Option<std::time::Instant>,

    // Global tag filter
    pub tag_filter: Option<String>,
    /// True when the tag filter input prompt is active.
    pub tag_filter_input: bool,
    pub tag_filter_buf: String,
}

impl App {
    pub fn new(cfg: Config, store: Arc<dyn Store>, tag_filter: Option<String>) -> Self {
        let (msg_tx, msg_rx) = mpsc::channel();
        let theme = Theme::gruvbox_dark();
        let data_dir = cfg.data_dir.clone();
        let store_clone = store.clone();

        Self {
            active_tab: ActiveTab::Dashboard,
            width: 0,
            height: 0,
            theme,

            dashboard: Dashboard::new(store.clone()),
            tasks_tab: TasksTab::new(store.clone()),
            calendar: CalendarView::new(store.clone()),
            notes: NoteView::new(store.clone()),
            people: PeopleView::new(store.clone()),

            sink: SinkOverlay::new(store.clone()),
            search: SearchOverlay::new(store.clone()),
            palette: CommandPalette::new(),
            help: HelpOverlay::new(),

            editor: None,
            show_editor: false,

            date_picker: DatePickerOverlay::new(),
            show_sink: false,
            show_search: false,
            show_palette: false,
            show_help: false,
            show_date_picker: false,
            date_picker_context: None,

            store: store_clone,
            data_dir,

            msg_tx,
            msg_rx,

            error_flash: None,
            error_clear_at: None,

            tag_filter,
            tag_filter_input: false,
            tag_filter_buf: String::new(),
        }
    }

    pub fn handle_event<B: Backend>(
        &mut self,
        event: &Event,
        terminal: &mut Terminal<B>,
    ) -> Option<AppMessage> {
        use crossterm::event::Event as CEvent;

        // Clear expired error flash
        if let Some(clear_at) = self.error_clear_at {
            if std::time::Instant::now() >= clear_at {
                self.error_flash = None;
                self.error_clear_at = None;
            }
        }

        match event {
            CEvent::Resize(w, h) => {
                self.width = *w;
                self.height = *h;
                let content_h = h.saturating_sub(4).max(1);
                let msg = AppMessage::Resize { width: *w, height: content_h };
                self.broadcast_message(&msg);
                // Resize embedded nvim to match the new content area.
                if self.show_editor {
                    if let Some(ref editor) = self.editor {
                        editor.resize(*w, content_h);
                    }
                }
                None
            }
            CEvent::Key(key_event) => self.handle_key(*key_event, terminal),
            _ => None,
        }
    }

    fn handle_key<B: Backend>(
        &mut self,
        key: crossterm::event::KeyEvent,
        terminal: &mut Terminal<B>,
    ) -> Option<AppMessage> {
        use crossterm::event::{KeyCode, KeyModifiers};

        // Inline nvim editor takes priority over everything else.
        if self.show_editor {
            if let Some(ref mut editor) = self.editor {
                editor.handle_key_event(&key);
            }
            return None;
        }

        // Tag filter input mode
        if self.tag_filter_input {
            match key.code {
                KeyCode::Enter => {
                    let buf = self.tag_filter_buf.trim().to_string();
                    self.tag_filter_input = false;
                    if buf.is_empty() {
                        self.set_tag_filter(None);
                    } else {
                        // Strip leading # if present
                        let tag = buf.strip_prefix('#').unwrap_or(&buf).to_string();
                        self.set_tag_filter(Some(tag));
                    }
                    return None;
                }
                KeyCode::Esc => {
                    self.tag_filter_input = false;
                    self.tag_filter_buf.clear();
                    return None;
                }
                KeyCode::Backspace => {
                    self.tag_filter_buf.pop();
                    return None;
                }
                KeyCode::Char(c) => {
                    self.tag_filter_buf.push(c);
                    return None;
                }
                _ => return None,
            }
        }

        // Esc dismisses help
        if key.code == KeyCode::Esc && self.show_help {
            self.show_help = false;
            return None;
        }

        // Ctrl+K toggles command palette
        if key.code == KeyCode::Char('k') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.show_palette = !self.show_palette;
            self.show_help = false;
            self.show_sink = false;
            self.show_search = false;
            return None;
        }

        // Ctrl+T toggles tag filter prompt
        if key.code == KeyCode::Char('t') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if self.tag_filter.is_some() {
                // Clear existing filter
                self.set_tag_filter(None);
            } else {
                // Open input prompt
                self.tag_filter_input = true;
                self.tag_filter_buf.clear();
            }
            return None;
        }

        // Forward to active overlay
        if self.show_palette {
            if let Some(msg) = self.palette.handle_event(&crossterm::event::Event::Key(key)) {
                return self.handle_app_message(msg, terminal);
            }
            return None;
        }
        if self.show_search {
            if let Some(msg) = self.search.handle_event(&crossterm::event::Event::Key(key)) {
                return self.handle_app_message(msg, terminal);
            }
            return None;
        }
        if self.show_sink {
            if let Some(msg) = self.sink.handle_event(&crossterm::event::Event::Key(key)) {
                return self.handle_app_message(msg, terminal);
            }
            return None;
        }
        if self.show_date_picker {
            if let Some(msg) = self.date_picker.handle_event(&crossterm::event::Event::Key(key)) {
                return self.handle_app_message(msg, terminal);
            }
            return None;
        }

        // Check if active view captures input
        if self.active_view_captures_input() {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return Some(AppMessage::Quit);
            }
            let msg = self.forward_to_active_tab(&crossterm::event::Event::Key(key));
            if let Some(m) = msg {
                return self.handle_app_message(m, terminal);
            }
            return None;
        }

        // Global keys
        match key.code {
            KeyCode::Char('q') => return Some(AppMessage::Quit),
            KeyCode::Char('D') => {
                self.active_tab = ActiveTab::Dashboard;
                self.close_overlays();
                self.broadcast_message(&AppMessage::Reload);
            }
            KeyCode::Char('T') => {
                self.active_tab = ActiveTab::Tasks;
                self.close_overlays();
                self.broadcast_message(&AppMessage::Reload);
            }
            KeyCode::Char('C') => {
                self.active_tab = ActiveTab::Calendar;
                self.close_overlays();
                self.broadcast_message(&AppMessage::Reload);
            }
            KeyCode::Char('N') => {
                self.active_tab = ActiveTab::Notes;
                self.close_overlays();
                self.broadcast_message(&AppMessage::Reload);
            }
            KeyCode::Char('P') => {
                self.active_tab = ActiveTab::People;
                self.close_overlays();
                self.people.on_tab_entered();
                self.broadcast_message(&AppMessage::Reload);
            }
            KeyCode::Char('?') => {
                self.show_help = !self.show_help;
                self.show_sink = false;
                self.show_search = false;
                self.show_palette = false;
            }
            KeyCode::Char('s') => {
                let was_open = self.show_sink;
                self.show_sink = !self.show_sink;
                self.show_help = false;
                self.show_search = false;
                self.show_palette = false;
                if self.show_sink && !was_open {
                    self.sink.handle_message(&AppMessage::OpenSink);
                }
            }
            KeyCode::Char('/') => {
                let was_open = self.show_search;
                self.show_search = !self.show_search;
                self.show_help = false;
                self.show_sink = false;
                self.show_palette = false;
                if self.show_search && !was_open {
                    self.search.handle_message(&AppMessage::OpenSearch);
                }
            }
            _ => {
                let msg = self.forward_to_active_tab(&crossterm::event::Event::Key(key));
                if let Some(m) = msg {
                    return self.handle_app_message(m, terminal);
                }
            }
        }
        None
    }

    fn handle_app_message<B: Backend>(
        &mut self,
        msg: AppMessage,
        _terminal: &mut Terminal<B>,
    ) -> Option<AppMessage> {
        match msg {
            AppMessage::Quit => Some(AppMessage::Quit),
            AppMessage::CloseSink => {
                self.show_sink = false;
                None
            }
            AppMessage::CloseSearch => {
                self.show_search = false;
                None
            }
            AppMessage::ClosePalette => {
                self.show_palette = false;
                None
            }
            AppMessage::CloseOverlays => {
                self.close_overlays();
                None
            }
            AppMessage::Error(msg) => {
                self.error_flash = Some(msg);
                self.error_clear_at = Some(
                    std::time::Instant::now() + std::time::Duration::from_secs(3),
                );
                None
            }
            AppMessage::OpenNoteEditor(id) => {
                // Ensure the note exists in the store before opening.
                if self.store.get_note(&id).is_err() {
                    let now = chrono::Utc::now();
                    let dir = std::env::current_dir()
                        .map(|d| d.display().to_string())
                        .unwrap_or_default();
                    let note = crate::domain::Note {
                        id: id.clone(),
                        title: String::new(),
                        created_at: now,
                        updated_at: now,
                        private: false,
                        pinned: false,
                        archived: false,
                        created_dir: dir,
                        refs: crate::domain::Refs::default(),
                        body: String::new(),
                    };
                    if let Err(e) = self.store.save_note(&note) {
                        self.error_flash = Some(format!("Failed to create note: {e}"));
                        return None;
                    }
                }
                let title = self.store.get_note(&id)
                    .map(|n| n.title.clone())
                    .unwrap_or_default();
                self.open_nvim_overlay(id, crate::domain::EntityKind::Note, title);
                None
            }
            AppMessage::OpenTaskEditor(id) => {
                let title = match self.store.get_task(&id) {
                    Ok(t) => t.title.clone(),
                    Err(e) => {
                        self.error_flash = Some(format!("Failed to load task: {e}"));
                        return None;
                    }
                };
                self.open_nvim_overlay(id, crate::domain::EntityKind::Task, title);
                None
            }
            AppMessage::OpenInlineEditor { kind, id } => {
                let title = match kind {
                    crate::domain::EntityKind::Note => {
                        self.store.get_note(&id).map(|n| n.title.clone()).unwrap_or_default()
                    }
                    crate::domain::EntityKind::Task => {
                        self.store.get_task(&id).map(|t| t.title.clone()).unwrap_or_default()
                    }
                    crate::domain::EntityKind::Agenda => {
                        self.store.get_agenda(&id).map(|a| a.title.clone()).unwrap_or_default()
                    }
                    _ => return None,
                };
                self.open_nvim_overlay(id, kind, title);
                None
            }
            AppMessage::EditorClosed => {
                self.show_editor = false;
                self.editor = None;
                self.broadcast_message(&AppMessage::Reload);
                None
            }
            AppMessage::Reload => {
                self.broadcast_message(&AppMessage::Reload);
                None
            }
            AppMessage::EditEntity { kind, id } => {
                use crate::domain::EntityKind;
                match kind {
                    EntityKind::Note => {
                        return self.handle_app_message(AppMessage::OpenNoteEditor(id), _terminal);
                    }
                    EntityKind::Task => {
                        self.active_tab = ActiveTab::Tasks;
                        self.close_overlays();
                        self.tasks_tab.handle_message(&AppMessage::EditEntity { kind, id });
                    }
                    _ => {}
                }
                None
            }
            AppMessage::NavigatePerson(ref slug) => {
                self.active_tab = ActiveTab::People;
                self.close_overlays();
                self.people.handle_message(&AppMessage::NavigatePerson(slug.clone()));
                None
            }
            AppMessage::NavigateRef(ref eref) => {
                use crate::domain::EntityKind;
                self.show_search = false;
                self.close_overlays();
                match eref.kind {
                    EntityKind::Agenda => {
                        self.active_tab = ActiveTab::People;
                        self.people.on_tab_entered();
                    }
                    EntityKind::Tag => {
                        self.active_tab = ActiveTab::Tasks;
                        self.tasks_tab.handle_message(&AppMessage::Reload);
                    }
                    _ => {}
                }
                None
            }
            AppMessage::PaletteAction(ref action) if action == "filter-tag" => {
                self.show_palette = false;
                self.tag_filter_input = true;
                self.tag_filter_buf = self.tag_filter.as_deref().unwrap_or("").to_string();
                None
            }
            AppMessage::PaletteAction(ref action) => {
                let action = action.clone();
                self.show_palette = false;
                match action.as_str() {
                    "dashboard" => {
                        self.active_tab = ActiveTab::Dashboard;
                        self.close_overlays();
                        self.dashboard.handle_message(&AppMessage::Reload);
                    }
                    "tasks" => {
                        self.active_tab = ActiveTab::Tasks;
                        self.close_overlays();
                        self.tasks_tab.handle_message(&AppMessage::Reload);
                    }
                    "calendar" => {
                        self.active_tab = ActiveTab::Calendar;
                        self.close_overlays();
                        self.calendar.handle_message(&AppMessage::Reload);
                    }
                    "notes" => {
                        self.active_tab = ActiveTab::Notes;
                        self.close_overlays();
                        self.notes.handle_message(&AppMessage::Reload);
                    }
                    "people" => {
                        self.active_tab = ActiveTab::People;
                        self.close_overlays();
                        self.people.on_tab_entered();
                    }
                    "search" => {
                        self.close_overlays();
                        self.show_search = true;
                        self.search.handle_message(&AppMessage::OpenSearch);
                    }
                    "sink" => {
                        self.close_overlays();
                        self.show_sink = true;
                        self.sink.handle_message(&AppMessage::OpenSink);
                    }
                    "quit" => {
                        return Some(AppMessage::Quit);
                    }
                    _ => {}
                }
                None
            }
            AppMessage::TagFilterChanged(_) => {
                // Handled by broadcast; should not reach here directly.
                None
            }
            AppMessage::OpenDatePicker { date, context } => {
                self.date_picker.open(date);
                self.date_picker_context = Some(context);
                self.show_date_picker = true;
                None
            }
            AppMessage::DatePickerConfirm(date) => {
                self.show_date_picker = false;
                if let Some(ctx) = self.date_picker_context.take() {
                    match ctx {
                        DatePickerContext::TaskDue(id) => {
                            if let Ok(mut task) = self.store.get_task(&id) {
                                task.due_date = Some(date);
                                task.updated_at = chrono::Utc::now();
                                if let Err(e) = self.store.save_task(&task) {
                                    return Some(AppMessage::Error(format!("Save failed: {}", e)));
                                }
                                self.broadcast_reload();
                            }
                        }
                        DatePickerContext::AgendaDate(id) => {
                            if let Ok(mut agenda) = self.store.get_agenda(&id) {
                                agenda.date = date;
                                agenda.updated_at = chrono::Utc::now();
                                if let Err(e) = self.store.save_agenda(&agenda) {
                                    return Some(AppMessage::Error(format!("Save failed: {}", e)));
                                }
                                self.broadcast_reload();
                            }
                        }
                    }
                }
                None
            }
            AppMessage::DatePickerCancel => {
                self.show_date_picker = false;
                self.date_picker_context = None;
                None
            }
            _ => None,
        }
    }

    fn active_view_captures_input(&self) -> bool {
        match self.active_tab {
            ActiveTab::Dashboard => self.dashboard.captures_input(),
            ActiveTab::Tasks => self.tasks_tab.captures_input(),
            ActiveTab::Calendar => self.calendar.captures_input(),
            ActiveTab::Notes => self.notes.captures_input(),
            ActiveTab::People => self.people.captures_input(),
        }
    }

    fn forward_to_active_tab(&mut self, event: &Event) -> Option<AppMessage> {
        match self.active_tab {
            ActiveTab::Dashboard => self.dashboard.handle_event(event),
            ActiveTab::Tasks => self.tasks_tab.handle_event(event),
            ActiveTab::Calendar => self.calendar.handle_event(event),
            ActiveTab::Notes => self.notes.handle_event(event),
            ActiveTab::People => self.people.handle_event(event),
        }
    }

    /// Send Reload to all views. Called at startup and on R key.
    /// Also broadcasts the current tag filter so views can apply it.
    pub fn broadcast_reload(&mut self) {
        let filter_msg = AppMessage::TagFilterChanged(self.tag_filter.clone());
        self.broadcast_message(&filter_msg);
        self.broadcast_message(&AppMessage::Reload);
    }

    /// Set the global tag filter and broadcast to all views.
    fn set_tag_filter(&mut self, filter: Option<String>) {
        let msg = AppMessage::TagFilterChanged(filter.clone());
        self.tag_filter = filter;
        self.broadcast_message(&msg);
        self.broadcast_message(&AppMessage::Reload);
    }

    fn broadcast_message(&mut self, msg: &AppMessage) {
        self.dashboard.handle_message(msg);
        self.tasks_tab.handle_message(msg);
        self.calendar.handle_message(msg);
        self.notes.handle_message(msg);
        self.people.handle_message(msg);
        self.sink.handle_message(msg);
        self.search.handle_message(msg);
    }

    fn close_overlays(&mut self) {
        self.show_help = false;
        self.show_sink = false;
        self.show_search = false;
        self.show_palette = false;
        self.show_date_picker = false;
        self.date_picker_context = None;
    }

    pub fn process_pending_messages(&mut self) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            match msg {
                AppMessage::Error(e) => {
                    self.error_flash = Some(e);
                    self.error_clear_at = Some(
                        std::time::Instant::now() + std::time::Duration::from_secs(3),
                    );
                }
                AppMessage::Reload => {
                    self.broadcast_message(&AppMessage::Reload);
                }
                _ => {}
            }
        }

        // Poll the embedded nvim overlay for async events (screen updates,
        // BufWritePost saves, nvim process exit).
        if self.show_editor {
            if let Some(ref mut editor) = self.editor {
                if let Some(AppMessage::EditorClosed) = editor.poll() {
                    self.show_editor = false;
                    self.editor = None;
                    self.broadcast_message(&AppMessage::Reload);
                }
            }
        }
    }

    /// Open the embedded nvim overlay for the given entity.
    fn open_nvim_overlay(&mut self, id: String, kind: crate::domain::EntityKind, title: String) {
        // Fall back to the real terminal size if width/height haven't been set
        // yet by a Resize event (they start at 0).
        let (w, h) = if self.width > 0 && self.height > 0 {
            (self.width, self.height)
        } else {
            crossterm::terminal::size().unwrap_or((80, 24))
        };
        let content_h = h.saturating_sub(4).max(1);
        match NvimOverlay::new(
            id,
            kind,
            title,
            self.store.clone(),
            self.data_dir.clone(),
            w,
            content_h,
        ) {
            Ok(overlay) => {
                self.editor = Some(overlay);
                self.show_editor = true;
                self.close_overlays();
            }
            Err(e) => {
                self.error_flash = Some(format!("Failed to open nvim: {e}"));
                self.error_clear_at = Some(
                    std::time::Instant::now() + std::time::Duration::from_secs(5),
                );
            }
        }
    }

    pub fn draw(&self, frame: &mut Frame) {
        use ratatui::layout::{Layout, Constraint, Direction};
        use ratatui::widgets::Paragraph;

        let area = frame.area();
        if area.width == 0 || area.height < 5 {
            let msg = Paragraph::new("Terminal too small. Please resize.")
                .style(self.theme.warning);
            frame.render_widget(msg, area);
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),  // tab bar
                Constraint::Length(1),  // separator
                Constraint::Min(1),    // content
                Constraint::Length(1),  // separator
                Constraint::Length(1),  // status bar
            ])
            .split(area);

        self.render_tab_bar(frame, chunks[0]);
        self.render_separator(frame, chunks[1]);
        self.render_content(frame, chunks[2]);
        self.render_separator(frame, chunks[3]);
        self.render_status_bar(frame, chunks[4]);
    }

    fn render_tab_bar(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::{Line, Span};

        let logo_style = Style::default()
            .fg(Color::Rgb(0x68, 0x9d, 0x6a))
            .add_modifier(Modifier::BOLD);
        let ornament_style = Style::default()
            .fg(self.theme.cursor);
        let mut spans = vec![
            Span::raw(" "),
            Span::styled("❖", ornament_style),
            Span::raw(" "),
            Span::styled("CRUMBS", logo_style),
            Span::raw(" "),
            Span::styled("❖", ornament_style),
            Span::styled(" │ ", self.theme.border),
        ];
        for tab in ActiveTab::ALL {
            let label = if area.width < 80 {
                tab.short_label()
            } else {
                tab.label()
            };
            let style = if tab == self.active_tab {
                self.theme.tab_active
            } else {
                self.theme.tab_inactive
            };
            spans.push(Span::styled(label, style));
            spans.push(Span::raw("  "));
        }

        let line = Line::from(spans);
        frame.render_widget(ratatui::widgets::Paragraph::new(line), area);
    }

    fn render_separator(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let sep = "─".repeat(area.width as usize);
        let p = ratatui::widgets::Paragraph::new(sep).style(self.theme.border);
        frame.render_widget(p, area);
    }

    fn render_content(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        // Inline editor takes priority over everything.
        if self.show_editor {
            if let Some(ref editor) = self.editor {
                editor.draw(frame, area, &self.theme);
                return;
            }
        }

        if self.show_help {
            self.help.draw(frame, area, &self.theme);
            return;
        }
        if self.show_sink {
            self.sink.draw(frame, area, &self.theme);
            return;
        }
        if self.show_palette {
            self.palette.draw(frame, area, &self.theme);
            return;
        }
        if self.show_search {
            self.search.draw(frame, area, &self.theme);
            return;
        }

        match self.active_tab {
            ActiveTab::Dashboard => self.dashboard.draw(frame, area, &self.theme),
            ActiveTab::Tasks => self.tasks_tab.draw(frame, area, &self.theme),
            ActiveTab::Calendar => self.calendar.draw(frame, area, &self.theme),
            ActiveTab::Notes => self.notes.draw(frame, area, &self.theme),
            ActiveTab::People => self.people.draw(frame, area, &self.theme),
        }

        // Date picker floats on top of the active view.
        if self.show_date_picker {
            self.date_picker.draw(frame, area, &self.theme);
        }
    }

    fn render_status_bar(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        use ratatui::text::{Line, Span};

        if let Some(ref err) = self.error_flash {
            let line = Line::from(Span::styled(
                format!(" {}", err),
                self.theme.error,
            ));
            frame.render_widget(ratatui::widgets::Paragraph::new(line), area);
            return;
        }

        // Tag filter input prompt
        if self.tag_filter_input {
            let line = Line::from(vec![
                Span::styled(" Filter by tag: ", self.theme.accent),
                Span::styled(&self.tag_filter_buf, self.theme.title),
                Span::styled("_", self.theme.accent),
            ]);
            frame.render_widget(ratatui::widgets::Paragraph::new(line), area);
            return;
        }

        let mut spans = Vec::new();

        // Leading space
        spans.push(Span::styled(" ", self.theme.dim));

        // Tag filter indicator
        if let Some(ref tag) = self.tag_filter {
            spans.push(Span::styled(
                format!("#{} \u{2717}", tag),
                self.theme.warning,
            ));
            spans.push(Span::styled("  ", self.theme.dim));
        }

        // Hint pairs: key in accent, description in dim, separated by "  "
        for (i, (key, desc)) in self.active_tab.status_hints().iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("  ", self.theme.dim));
            }
            spans.push(Span::styled(*key, self.theme.accent));
            spans.push(Span::styled(format!(": {}", desc), self.theme.dim));
        }

        let line = Line::from(spans);
        frame.render_widget(ratatui::widgets::Paragraph::new(line), area);
    }
}
