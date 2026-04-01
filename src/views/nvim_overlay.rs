//! Inline nvim overlay — renders an embedded `nvim --embed` process inside
//! the Ratatui layout, replacing the old `tui-textarea`-based `EditorOverlay`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::app::message::AppMessage;
use crate::app::theme::Theme;
use crate::domain::{Agenda, EntityKind, Note, Priority, Task};
use crate::parser::{extract_mentions, extract_tags};
use crate::store::Store;
use super::nvim_bridge::{key_to_nvim_input, NvimBridge};

// ── MetadataSnapshot ─────────────────────────────────────────────

/// Snapshot of entity metadata displayed as a formatted header above the nvim editor.
enum MetadataSnapshot {
    Task(Box<Task>),
    Note(Box<Note>),
    Agenda(Box<Agenda>),
}

impl MetadataSnapshot {
    /// Number of rows the header occupies (title + metadata rows + separator).
    fn header_height(&self) -> u16 {
        match self {
            // title + 8 fields (status/priority/due/tags/private/pinned/created/modified) + sep
            MetadataSnapshot::Task(_) => 10,
            // title + 5 fields (tags/private/pinned/created/modified) + sep
            MetadataSnapshot::Note(_) => 7,
            // title + 4 fields (person/date/created/modified) + sep
            MetadataSnapshot::Agenda(_) => 6,
        }
    }
}

// ── NvimOverlay ──────────────────────────────────────────────────

pub struct NvimOverlay {
    bridge: NvimBridge,

    entity_id: String,
    pub entity_kind: EntityKind,

    /// Absolute path of the file open in nvim.
    file_path: PathBuf,
    /// True for tasks/agendas whose file is a temp .md that we created.
    is_temp_file: bool,

    store: Arc<dyn Store>,

    /// Cached entity data for the metadata header display.
    metadata: MetadataSnapshot,
    /// Rows reserved for the metadata header (data rows + separator).
    header_height: u16,
}

impl NvimOverlay {
    /// Create and open a new nvim overlay for the given entity.
    ///
    /// `width` / `height` should be the content area dimensions. The overlay
    /// reserves `header_height` rows at the top for the metadata display and
    /// gives the remaining rows to nvim.
    pub fn new(
        entity_id: String,
        entity_kind: EntityKind,
        _title: String,
        store: Arc<dyn Store>,
        data_dir: PathBuf,
        width: u16,
        height: u16,
    ) -> anyhow::Result<Self> {
        let (file_path, is_temp_file, metadata) = prepare_file(
            &entity_id,
            &entity_kind,
            &store,
            &data_dir,
        )?;

        let header_height = metadata.header_height();
        let nvim_height = height.saturating_sub(header_height).max(1);
        let bridge = NvimBridge::spawn(width, nvim_height)?;
        bridge.open_file(file_path.clone());

        Ok(Self {
            bridge,
            entity_id,
            entity_kind,
            file_path,
            is_temp_file,
            store,
            metadata,
            header_height,
        })
    }

    // ── Rendering ─────────────────────────────────────────────────

    pub fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        frame.render_widget(Clear, area);

        // Split into header area (metadata) and editor area (nvim).
        let hh = self.header_height;
        let (header_area, editor_area) = if area.height > hh {
            (
                Rect { height: hh, ..area },
                Rect { y: area.y + hh, height: area.height - hh, ..area },
            )
        } else {
            (Rect { height: 0, ..area }, area)
        };

        if header_area.height > 0 {
            self.draw_header(frame, header_area, theme);
        }

        // Render the nvim virtual grid into editor_area.
        let screen = self.bridge.screen();
        let buf = frame.buffer_mut();

        for (row_idx, row) in screen.grid.iter().enumerate() {
            let y = editor_area.y + row_idx as u16;
            if y >= editor_area.y + editor_area.height {
                break;
            }
            for (col_idx, cell) in row.iter().enumerate() {
                let x = editor_area.x + col_idx as u16;
                if x >= editor_area.x + editor_area.width {
                    break;
                }
                let style = screen.highlights.get(&cell.hl_id).copied().unwrap_or_default();
                buf.set_string(x, y, &cell.text, style);
            }
        }

        // Cursor — render with reversed video so it's visible regardless of
        // the nvim colorscheme.
        let (crow, ccol) = screen.cursor;
        let cx = editor_area.x + ccol;
        let cy = editor_area.y + crow;
        if cx < editor_area.x + editor_area.width && cy < editor_area.y + editor_area.height {
            if let Some(cell) = buf.cell_mut(ratatui::layout::Position::new(cx, cy)) {
                cell.set_style(Style::default().add_modifier(Modifier::REVERSED));
            }
        }
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        // Collect all rows, then render only as many as fit in the area.
        let rows: Vec<Line> = match &self.metadata {
            MetadataSnapshot::Task(task) => {
                let status_style = theme.status_fg(&task.status);
                let status_val = Line::from(vec![
                    Span::styled(task.status.icon().to_string(), status_style),
                    Span::raw(" "),
                    Span::styled(task.status.label().to_string(), status_style),
                ]);
                let priority_val = if task.priority.is_none() {
                    Line::from(Span::styled("—", theme.dim))
                } else {
                    let ps = match task.priority {
                        Priority::High => theme.priority_high,
                        Priority::Medium => theme.priority_medium,
                        Priority::Low => theme.priority_low,
                        Priority::None => theme.dim,
                    };
                    Line::from(Span::styled(task.priority.label().to_string(), ps))
                };
                let due_val = match task.due_date {
                    Some(d) => Line::from(Span::styled(
                        d.format("%Y-%m-%d").to_string(),
                        theme.date,
                    )),
                    None => Line::from(Span::styled("—", theme.dim)),
                };
                vec![
                    title_row(task.title.clone(), theme),
                    meta_row("󰄲", "Status",   status_val,                       theme),
                    meta_row("󰓅", "Priority", priority_val,                     theme),
                    meta_row("󰃰", "Due",      due_val,                          theme),
                    meta_row("󰓹", "Tags",     tags_val(&task.refs.tags, theme), theme),
                    meta_row("󰌾", "Private",  bool_val(task.private, theme),    theme),
                    meta_row("󰐃", "Pinned",   bool_val(task.pinned, theme),     theme),
                    meta_row("󰃳", "Created",  datetime_val(task.created_at),    theme),
                    meta_row("󰢧", "Modified", datetime_val(task.updated_at),    theme),
                    separator_row(area.width, theme),
                ]
            }
            MetadataSnapshot::Note(note) => {
                vec![
                    title_row(note.title.clone(), theme),
                    meta_row("󰓹", "Tags",     tags_val(&note.refs.tags, theme), theme),
                    meta_row("󰌾", "Private",  bool_val(note.private, theme),    theme),
                    meta_row("󰐃", "Pinned",   bool_val(note.pinned, theme),     theme),
                    meta_row("󰃳", "Created",  datetime_val(note.created_at),    theme),
                    meta_row("󰢧", "Modified", datetime_val(note.updated_at),    theme),
                    separator_row(area.width, theme),
                ]
            }
            MetadataSnapshot::Agenda(agenda) => {
                vec![
                    title_row(agenda.title.clone(), theme),
                    meta_row("󰀄", "Person", Line::from(Span::styled(agenda.person_slug.clone(), theme.person)), theme),
                    meta_row("󰃭", "Date",   Line::from(Span::styled(agenda.date.format("%Y-%m-%d").to_string(), theme.date)), theme),
                    meta_row("󰃳", "Created",  datetime_val(agenda.created_at), theme),
                    meta_row("󰢧", "Modified", datetime_val(agenda.updated_at), theme),
                    separator_row(area.width, theme),
                ]
            }
        };

        for (i, row) in rows.into_iter().enumerate() {
            let y = area.y + i as u16;
            if y >= area.y + area.height {
                break;
            }
            frame.render_widget(
                Paragraph::new(row),
                Rect { y, height: 1, ..area },
            );
        }
    }

    // ── Event handling ────────────────────────────────────────────

    /// Called from `App::handle_key` when the overlay is active.
    /// Forwards all keys directly to nvim.
    pub fn handle_key_event(&mut self, key: &KeyEvent) -> Option<AppMessage> {
        let input = key_to_nvim_input(key);
        if !input.is_empty() {
            self.bridge.send_key(&input);
        }
        None
    }

    /// Poll for async nvim events (screen updates, saves, exits).
    /// Called every loop iteration from `App::process_pending_messages`.
    pub fn poll(&mut self) -> Option<AppMessage> {
        let (buf_written, exited) = {
            let mut s = self.bridge.screen();
            let bw = s.buf_written;
            let ex = s.exited;
            s.buf_written = false;
            (bw, ex)
        };

        if buf_written {
            if let Some(msg) = self.handle_buffer_written() {
                return Some(msg);
            }
        }

        if exited {
            self.cleanup();
            return Some(AppMessage::EditorClosed);
        }

        None
    }

    pub fn resize(&self, width: u16, height: u16) {
        self.bridge.resize(width, height.saturating_sub(self.header_height).max(1));
    }

    // ── Save helpers ──────────────────────────────────────────────

    /// Called when nvim signals `BufWritePost` — read the file back and
    /// update the entity body/description in the store.
    fn handle_buffer_written(&self) -> Option<AppMessage> {
        let result = match self.entity_kind {
            EntityKind::Note => self.save_note(),
            EntityKind::Task => self.save_task(),
            EntityKind::Agenda => self.save_agenda(),
            _ => Ok(()),
        };
        result.err().map(|e| AppMessage::Error(format!("Save failed: {e}")))
    }

    fn save_note(&self) -> anyhow::Result<()> {
        let body = std::fs::read_to_string(&self.file_path)?;
        let mut note = self.store.get_note(&self.entity_id)?;
        note.body = body.clone();
        note.updated_at = chrono::Utc::now();
        for m in extract_mentions(&body) {
            if !note.refs.people.contains(&m) {
                note.refs.people.push(m);
            }
        }
        for t in extract_tags(&body) {
            if !note.refs.tags.contains(&t) {
                note.refs.tags.push(t);
            }
        }
        self.store.save_note(&note)?;
        Ok(())
    }

    fn save_task(&self) -> anyhow::Result<()> {
        let body = std::fs::read_to_string(&self.file_path)?;
        let mut task = self.store.get_task(&self.entity_id)?;
        task.description = body.trim_end().to_string();
        task.updated_at = chrono::Utc::now();
        self.store.save_task(&task)?;
        Ok(())
    }

    fn save_agenda(&self) -> anyhow::Result<()> {
        let body = std::fs::read_to_string(&self.file_path)?;
        let mut agenda = self.store.get_agenda(&self.entity_id)?;
        agenda.body = body;
        agenda.updated_at = chrono::Utc::now();
        self.store.save_agenda(&agenda)?;
        Ok(())
    }

    fn cleanup(&self) {
        self.bridge.send_key(":qa!\n");
        if self.is_temp_file {
            let _ = std::fs::remove_file(&self.file_path);
        }
    }
}

// ── Header render helpers ─────────────────────────────────────────

/// Title row: bold, full width, indented.
fn title_row(title: String, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(title, theme.title),
    ])
}

/// A single metadata row: `  glyph  Key Name    value`.
///
/// The key column is fixed at 10 chars so values align across rows.
fn meta_row<'a>(glyph: &'static str, key: &'static str, value: Line<'a>, theme: &Theme) -> Line<'a> {
    let mut spans = vec![
        Span::styled("  ", Style::default()),
        Span::styled(glyph, theme.dim),
        Span::styled("  ", Style::default()),
        Span::styled(format!("{:<10}", key), theme.dim),
    ];
    spans.extend(value.spans);
    Line::from(spans)
}

fn separator_row(width: u16, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled("─".repeat(width as usize), theme.border))
}

fn bool_val(v: bool, theme: &Theme) -> Line<'static> {
    if v {
        Line::from(Span::styled("Yes", theme.success))
    } else {
        Line::from(Span::styled("No", theme.dim))
    }
}

fn tags_val<'a>(tags: &[String], theme: &Theme) -> Line<'a> {
    if tags.is_empty() {
        return Line::from(Span::styled("—", theme.dim));
    }
    let mut spans: Vec<Span> = Vec::new();
    for tag in tags {
        spans.push(Span::styled(format!("#{} ", tag), theme.topic));
    }
    Line::from(spans)
}

fn datetime_val(dt: chrono::DateTime<chrono::Utc>) -> Line<'static> {
    Line::from(dt.format("%Y-%m-%d %H:%M").to_string())
}

// ── File preparation ──────────────────────────────────────────────

/// Write only the body/description to a temp file and return the entity
/// snapshot for the metadata header.
fn prepare_file(
    entity_id: &str,
    entity_kind: &EntityKind,
    store: &Arc<dyn Store>,
    _data_dir: &Path,
) -> anyhow::Result<(PathBuf, bool, MetadataSnapshot)> {
    match entity_kind {
        EntityKind::Note => {
            let note = match store.get_note(entity_id) {
                Ok(n) => n,
                Err(_) => {
                    let now = chrono::Utc::now();
                    Note {
                        id: entity_id.to_string(),
                        title: String::new(),
                        created_at: now,
                        updated_at: now,
                        private: false,
                        pinned: false,
                        archived: false,
                        created_dir: std::env::current_dir()
                            .map(|d| d.display().to_string())
                            .unwrap_or_default(),
                        refs: crate::domain::Refs::default(),
                        body: String::new(),
                    }
                }
            };
            let tmp_path = std::env::temp_dir().join(format!("crumbs-note-{entity_id}.md"));
            std::fs::write(&tmp_path, &note.body)?;
            Ok((tmp_path, true, MetadataSnapshot::Note(Box::new(note))))
        }
        EntityKind::Task => {
            let task = store.get_task(entity_id)?;
            let tmp_path = std::env::temp_dir().join(format!("crumbs-task-{entity_id}.md"));
            std::fs::write(&tmp_path, &task.description)?;
            Ok((tmp_path, true, MetadataSnapshot::Task(Box::new(task))))
        }
        EntityKind::Agenda => {
            let agenda = store.get_agenda(entity_id)?;
            let tmp_path = std::env::temp_dir().join(format!("crumbs-agenda-{entity_id}.md"));
            std::fs::write(&tmp_path, &agenda.body)?;
            Ok((tmp_path, true, MetadataSnapshot::Agenda(Box::new(agenda))))
        }
        _ => Err(anyhow::anyhow!("unsupported entity kind for nvim overlay")),
    }
}
