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
use crate::parser::{extract_mentions, extract_topics};
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
        // Parse the full file so front-matter edits (including refs.topics) are honoured.
        let Ok(mut note) = crate::store::io::parse_note(&self.file_path) else { return };
        note.updated_at = chrono::Utc::now();

        // Also merge any new @mentions / #topics written inline in the body.
        let body = note.body.clone();
        for m in extract_mentions(&body) {
            if !note.refs.people.contains(&m) {
                note.refs.people.push(m);
            }
        }
        for t in extract_topics(&body) {
            if !note.refs.topics.contains(&t) {
                note.refs.topics.push(t);
            }
        }

        let _ = self.store.save_note(&note);
        let _ = self.store.rebuild_index();
    }

    fn save_task(&self) {
        let Ok(content) = std::fs::read_to_string(&self.file_path) else { return };
        let json_path = self.data_dir.join("tasks").join(format!("{}.json", self.entity_id));
        let _ = apply_task_frontmatter(&content, &json_path);
        let _ = self.store.rebuild_index();
    }

    fn save_agenda(&self) {
        let Ok(mut agenda) = crate::store::io::parse_agenda(&self.file_path) else { return };
        agenda.updated_at = chrono::Utc::now();
        let _ = self.store.save_agenda(&agenda);
        let _ = self.store.rebuild_index();
    }

    fn cleanup(&self) {
        // Signal nvim to quit cleanly before removing any temp file.
        self.bridge.send_key(":qa!\n");
        if self.is_temp_file {
            let _ = std::fs::remove_file(&self.file_path);
        }
    }
}

// ── File preparation ──────────────────────────────────────────────

fn prepare_file(
    entity_id: &str,
    entity_kind: &EntityKind,
    store: &Arc<dyn Store>,
    data_dir: &Path,
) -> anyhow::Result<(PathBuf, bool)> {
    match entity_kind {
        EntityKind::Note => {
            let path = data_dir.join("notes").join(format!("{}.md", entity_id));
            if !path.exists() {
                // Create a template so nvim has something to edit.
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let now = chrono::Utc::now().to_rfc3339();
                let dir = std::env::current_dir()
                    .map(|d| d.display().to_string())
                    .unwrap_or_default();
                let tmpl = format!(
                    "---\nid: {entity_id}\ntitle: \ncreated_at: {now}\nupdated_at: {now}\ncreated_dir: {dir}\nrefs:\n  people: []\n  topics: []\n---\n\n"
                );
                std::fs::write(&path, tmpl)?;
            }
            Ok((path, false))
        }
        EntityKind::Task => {
            let path = data_dir.join("tasks").join(format!("{}.md", entity_id));
            let task = store.get_task(entity_id)?;
            let tags_str = task.refs.topics.join(" ");
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
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)?;
            Ok((path, true))
        }
        EntityKind::Agenda => {
            let agenda = store.get_agenda(entity_id)?;
            // Write full frontmatter+body to a temp file (like notes) so the
            // user can edit metadata fields.  The store's canonical .md is left
            // untouched; save_agenda() parses this temp file and writes back
            // through the store on every :w.
            let tmp_path = std::env::temp_dir().join(format!("crumbs-agenda-{entity_id}.md"));
            crate::store::io::write_agenda(&tmp_path, &agenda)?;
            Ok((tmp_path, true))
        }
        _ => Err(anyhow::anyhow!("unsupported entity kind for nvim overlay")),
    }
}

// ── Parse helpers ─────────────────────────────────────────────────

/// Parse a task's temp `.md` front matter and apply changes to the task's
/// `.json` file on disk.
fn apply_task_frontmatter(content: &str, json_path: &Path) -> anyhow::Result<()> {
    if !content.starts_with("---\n") {
        return Ok(());
    }
    let rest = &content[4..];
    let end_idx = rest.find("\n---\n").ok_or_else(|| anyhow::anyhow!("missing fm end"))?;

    let fm_text = &rest[..end_idx];
    let body = rest[end_idx + 5..].trim();

    let raw = std::fs::read_to_string(json_path)?;
    let mut task: serde_json::Value = serde_json::from_str(&raw)?;

    for line in fm_text.lines() {
        let line = line.trim();
        let Some(colon) = line.find(':') else { continue };
        let key = line[..colon].trim();
        let val = line[colon + 1..].trim();

        match key {
            "title" => {
                task["title"] = serde_json::Value::String(val.to_string());
            }
            "status" => {
                let normalized = crate::domain::TaskStatus::from_str_loose(val)
                    .map(|s| s.as_str().to_string())
                    .unwrap_or_else(|| val.to_string());
                task["status"] = serde_json::Value::String(normalized);
            }
            "due_date" => {
                if val.is_empty() || val == "null" {
                    task.as_object_mut().map(|m| m.remove("due_date"));
                } else {
                    task["due_date"] = serde_json::Value::String(val.to_string());
                }
            }
            "priority" => {
                task["priority"] = serde_json::Value::String(val.to_string());
            }
            "private" => {
                task["private"] = serde_json::Value::Bool(val == "true");
            }
            "tags" => {
                let topics: serde_json::Value = val
                    .split_whitespace()
                    .filter(|s| !s.is_empty())
                    .map(|s| serde_json::Value::String(s.trim_start_matches('#').to_string()))
                    .collect::<Vec<_>>()
                    .into();
                if let Some(refs) = task.get_mut("refs") {
                    refs["topics"] = topics;
                } else {
                    task["refs"] = serde_json::json!({ "topics": topics });
                }
            }
            _ => {} // id, created_at, updated_at are read-only
        }
    }

    if !body.is_empty() {
        task["description"] = serde_json::Value::String(body.to_string());
    } else {
        task.as_object_mut().map(|m| m.remove("description"));
    }
    task["updated_at"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());

    let data = serde_json::to_string_pretty(&task)?;
    std::fs::write(json_path, data)?;
    Ok(())
}

