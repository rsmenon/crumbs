//! Inline nvim overlay — renders an embedded `nvim --embed` process inside
//! the Ratatui layout, replacing the old `tui-textarea`-based `EditorOverlay`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Clear;
use ratatui::Frame;

use crate::app::message::AppMessage;
use crate::app::theme::Theme;
use crate::domain::EntityKind;
use crate::parser::{extract_mentions, extract_tags};
use crate::store::Store;
use super::nvim_bridge::{key_to_nvim_input, NvimBridge};

// ── NvimOverlay ──────────────────────────────────────────────────

pub struct NvimOverlay {
    bridge: NvimBridge,

    entity_id: String,
    pub entity_kind: EntityKind,
    #[allow(dead_code)]
    pub title: String,

    /// Absolute path of the file open in nvim.
    file_path: PathBuf,
    /// True for tasks/agendas whose file is a temp .md that we created.
    is_temp_file: bool,

    store: Arc<dyn Store>,
    data_dir: PathBuf,
}

impl NvimOverlay {
    /// Create and open a new nvim overlay for the given entity.
    ///
    /// `width` / `height` should be the content area dimensions (excluding the
    /// 2-row header+separator that the overlay draws itself).
    pub fn new(
        entity_id: String,
        entity_kind: EntityKind,
        title: String,
        store: Arc<dyn Store>,
        data_dir: PathBuf,
        width: u16,
        height: u16,
    ) -> anyhow::Result<Self> {
        let (file_path, is_temp_file) = prepare_file(
            &entity_id,
            &entity_kind,
            &store,
            &data_dir,
        )?;

        let bridge = NvimBridge::spawn(width, height.max(1))?;
        bridge.open_file(file_path.clone());

        Ok(Self {
            bridge,
            entity_id,
            entity_kind,
            title,
            file_path,
            is_temp_file,
            store,
            data_dir,
        })
    }

    // ── Rendering ─────────────────────────────────────────────────

    pub fn draw(&self, frame: &mut Frame, area: Rect, _theme: &Theme) {
        frame.render_widget(Clear, area);

        let screen = self.bridge.screen();

        // Give nvim the full content area — it renders its own statusline,
        // mode indicator, and file info.
        let content = area;
        let buf = frame.buffer_mut();

        for (row_idx, row) in screen.grid.iter().enumerate() {
            let y = content.y + row_idx as u16;
            if y >= content.y + content.height {
                break;
            }
            for (col_idx, cell) in row.iter().enumerate() {
                let x = content.x + col_idx as u16;
                if x >= content.x + content.width {
                    break;
                }
                let style = screen.highlights.get(&cell.hl_id).copied().unwrap_or_default();
                buf.set_string(x, y, &cell.text, style);
            }
        }

        // Cursor — render with reversed video so it's visible regardless of
        // the nvim colorscheme.
        let (crow, ccol) = screen.cursor;
        let cx = content.x + ccol;
        let cy = content.y + crow;
        if cx < content.x + content.width && cy < content.y + content.height {
            if let Some(cell) = buf.cell_mut(ratatui::layout::Position::new(cx, cy)) {
                cell.set_style(Style::default().add_modifier(Modifier::REVERSED));
            }
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
            self.handle_buffer_written();
        }

        if exited {
            self.cleanup();
            return Some(AppMessage::EditorClosed);
        }

        None
    }

    pub fn resize(&self, width: u16, height: u16) {
        self.bridge.resize(width, height);
    }

    // ── Save helpers ──────────────────────────────────────────────

    /// Called when nvim signals `BufWritePost` — read the file back and
    /// update the in-memory store.
    fn handle_buffer_written(&self) {
        match self.entity_kind {
            EntityKind::Note => self.save_note(),
            EntityKind::Task => self.save_task(),
            EntityKind::Agenda => self.save_agenda(),
            _ => {}
        }
    }

    fn save_note(&self) {
        // Parse the temp file so front-matter edits (including refs.tags) are honoured.
        let Ok(mut note) = crate::store::io::parse_note(&self.file_path) else { return };
        note.updated_at = chrono::Utc::now();

        // Also merge any new @mentions / #tags written inline in the body.
        let body = note.body.clone();
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

        let _ = self.store.save_note(&note);
    }

    fn save_task(&self) {
        let Ok(content) = std::fs::read_to_string(&self.file_path) else { return };
        let _ = self.apply_task_frontmatter_to_store(&content);
    }

    fn save_agenda(&self) {
        let Ok(mut agenda) = crate::store::io::parse_agenda(&self.file_path) else { return };
        agenda.updated_at = chrono::Utc::now();
        let _ = self.store.save_agenda(&agenda);
    }

    fn cleanup(&self) {
        // Signal nvim to quit cleanly before removing any temp file.
        self.bridge.send_key(":qa!\n");
        if self.is_temp_file {
            let _ = std::fs::remove_file(&self.file_path);
        }
    }

    /// Parse the temp .md front matter and save the task back through the store.
    fn apply_task_frontmatter_to_store(&self, content: &str) -> anyhow::Result<()> {
        if !content.starts_with("---\n") {
            return Ok(());
        }
        let rest = &content[4..];
        let end_idx = rest.find("\n---\n").ok_or_else(|| anyhow::anyhow!("missing fm end"))?;

        let fm_text = &rest[..end_idx];
        let body = rest[end_idx + 5..].trim();

        let mut task = self.store.get_task(&self.entity_id)?;

        for line in fm_text.lines() {
            let line = line.trim();
            let Some(colon) = line.find(':') else { continue };
            let key = line[..colon].trim();
            let val = line[colon + 1..].trim();

            match key {
                "title" => {
                    task.title = val.to_string();
                }
                "status" => {
                    if let Some(s) = crate::domain::TaskStatus::from_str_loose(val) {
                        task.status = s;
                    }
                }
                "due_date" => {
                    if val.is_empty() || val == "null" {
                        task.due_date = None;
                    } else {
                        task.due_date = chrono::NaiveDate::parse_from_str(val, "%Y-%m-%d").ok();
                    }
                }
                "priority" => {
                    task.priority = match val.to_lowercase().as_str() {
                        "low" => crate::domain::Priority::Low,
                        "medium" | "med" => crate::domain::Priority::Medium,
                        "high" => crate::domain::Priority::High,
                        _ => crate::domain::Priority::None,
                    };
                }
                "private" => {
                    task.private = val == "true";
                }
                "tags" => {
                    task.refs.tags = val
                        .split_whitespace()
                        .filter(|s| !s.is_empty())
                        .map(|s| s.trim_start_matches('#').to_string())
                        .collect();
                }
                _ => {} // id, created_at, updated_at are read-only
            }
        }

        if !body.is_empty() {
            task.description = body.to_string();
        } else {
            task.description = String::new();
        }
        task.updated_at = chrono::Utc::now();

        self.store.save_task(&task)?;
        Ok(())
    }
}

// ── File preparation ──────────────────────────────────────────────

fn prepare_file(
    entity_id: &str,
    entity_kind: &EntityKind,
    store: &Arc<dyn Store>,
    _data_dir: &Path,
) -> anyhow::Result<(PathBuf, bool)> {
    match entity_kind {
        EntityKind::Note => {
            // Notes now use temp files like tasks/agendas.
            let note = match store.get_note(entity_id) {
                Ok(n) => n,
                Err(_) => {
                    // Create a blank note template
                    let now = chrono::Utc::now();
                    crate::domain::Note {
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
            crate::store::io::write_note(&tmp_path, &note)?;
            Ok((tmp_path, true))
        }
        EntityKind::Task => {
            let task = store.get_task(entity_id)?;
            let tags_str = task.refs.tags.join(" ");
            let tmp_path = std::env::temp_dir().join(format!("crumbs-task-{entity_id}.md"));
            let content = format!(
                "---\nid: {}\ntitle: {}\nstatus: {}\ndue_date: {}\npriority: {}\nprivate: {}\ntags: {}\ncreated_at: {}\nupdated_at: {}\n---\n\n{}",
                task.id,
                task.title,
                task.status.as_str(),
                task.due_date.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default().as_str(),
                task.priority.label(),
                task.private,
                tags_str,
                task.created_at.to_rfc3339(),
                task.updated_at.to_rfc3339(),
                task.description,
            );
            std::fs::write(&tmp_path, content)?;
            Ok((tmp_path, true))
        }
        EntityKind::Agenda => {
            let agenda = store.get_agenda(entity_id)?;
            let tmp_path = std::env::temp_dir().join(format!("crumbs-agenda-{entity_id}.md"));
            crate::store::io::write_agenda(&tmp_path, &agenda)?;
            Ok((tmp_path, true))
        }
        _ => Err(anyhow::anyhow!("unsupported entity kind for nvim overlay")),
    }
}

