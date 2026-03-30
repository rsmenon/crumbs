use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{params, Connection, Transaction};

use crate::domain::*;
use crate::parser::{extract_mentions, extract_tags};
use super::Store;

const SCHEMA: &str = include_str!("schema.sql");

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Open or create a SQLite database at `path`.
    pub fn new(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening database at {}", path.display()))?;

        // Run PRAGMAs before schema (journal_mode, foreign_keys, synchronous
        // are in the schema SQL but journal_mode must be set before any table
        // creation in some SQLite builds).
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;

        // Create tables if they don't exist.
        conn.execute_batch(SCHEMA)?;

        // Run incremental schema migrations.
        run_migrations(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

// ── Migrations ─────────────────────────────────────────────────────

fn run_migrations(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if version < 1 {
        // Migration 1: add ON UPDATE CASCADE to person_metadata and agendas.
        // Requires table recreation since SQLite does not support ALTER CONSTRAINT.
        conn.execute_batch("
            PRAGMA foreign_keys = OFF;
            BEGIN;

            CREATE TABLE person_metadata_new (
                person_slug TEXT NOT NULL REFERENCES persons(slug) ON DELETE CASCADE ON UPDATE CASCADE,
                key         TEXT NOT NULL,
                value       TEXT NOT NULL,
                PRIMARY KEY (person_slug, key)
            );
            INSERT OR IGNORE INTO person_metadata_new SELECT * FROM person_metadata;
            DROP TABLE person_metadata;
            ALTER TABLE person_metadata_new RENAME TO person_metadata;

            CREATE TABLE agendas_new (
                id          TEXT PRIMARY KEY,
                title       TEXT NOT NULL DEFAULT '',
                person_slug TEXT NOT NULL REFERENCES persons(slug) ON DELETE CASCADE ON UPDATE CASCADE,
                date        TEXT NOT NULL,
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL,
                body        TEXT NOT NULL DEFAULT ''
            );
            INSERT OR IGNORE INTO agendas_new SELECT * FROM agendas;
            DROP TABLE agendas;
            ALTER TABLE agendas_new RENAME TO agendas;

            CREATE INDEX IF NOT EXISTS idx_agendas_person ON agendas(person_slug);
            CREATE INDEX IF NOT EXISTS idx_agendas_date ON agendas(date);

            PRAGMA user_version = 1;
            COMMIT;
            PRAGMA foreign_keys = ON;
        ")?;
    }

    Ok(())
}

// ── Helper functions ───────────────────────────────────────────────

fn ensure_person_exists(tx: &Transaction, slug: &str) -> Result<()> {
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO persons (slug, created_at, pinned, archived) VALUES (?1, ?2, 0, 0)",
        params![slug, Utc::now().to_rfc3339()],
    )?;
    // Only add to FTS if a new row was created (INSERT OR IGNORE skips on conflict)
    if inserted > 0 {
        update_fts(tx, "person", slug, slug)?;
    }
    Ok(())
}

fn ensure_tag_exists(tx: &Transaction, slug: &str) -> Result<()> {
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO tags (slug, created_at) VALUES (?1, ?2)",
        params![slug, Utc::now().to_rfc3339()],
    )?;
    if inserted > 0 {
        update_fts(tx, "tag", slug, slug)?;
    }
    Ok(())
}

/// Replace `@old_slug` with `@new_slug` using word-boundary awareness.
///
/// A match is only replaced when the character immediately following the slug
/// is not alphanumeric or `-`, preventing `@foo` from corrupting `@foobar`.
fn replace_mention(text: &str, old_slug: &str, new_slug: &str) -> String {
    let pattern = format!("@{}", old_slug);
    let replacement = format!("@{}", new_slug);
    let mut result = String::with_capacity(text.len() + replacement.len());
    let mut pos = 0;
    while let Some(rel) = text[pos..].find(&pattern) {
        let abs = pos + rel;
        let after = abs + pattern.len();
        let boundary = text[after..].chars().next()
            .map(|c| !c.is_alphanumeric() && c != '-')
            .unwrap_or(true);
        if boundary {
            result.push_str(&text[pos..abs]);
            result.push_str(&replacement);
            pos = after;
        } else {
            result.push_str(&text[pos..abs + 1]);
            pos = abs + 1;
        }
    }
    result.push_str(&text[pos..]);
    result
}

/// Merge @mentions and #tags extracted from `text` into `refs` (additive, no duplicates).
fn merge_text_refs(refs: &mut Refs, text: &str) {
    for m in extract_mentions(text) {
        if !refs.people.contains(&m) {
            refs.people.push(m);
        }
    }
    for t in extract_tags(text) {
        if !refs.tags.contains(&t) {
            refs.tags.push(t);
        }
    }
}

fn save_entity_refs(tx: &Transaction, source_kind: &str, source_id: &str, refs: &Refs) -> Result<()> {
    // Delete existing refs for this entity
    tx.execute(
        "DELETE FROM entity_refs WHERE source_kind = ?1 AND source_id = ?2",
        params![source_kind, source_id],
    )?;

    // Insert person refs
    for person in &refs.people {
        ensure_person_exists(tx, person)?;
        tx.execute(
            "INSERT OR IGNORE INTO entity_refs (source_kind, source_id, target_kind, target_id) VALUES (?1, ?2, 'person', ?3)",
            params![source_kind, source_id, person],
        )?;
    }

    // Insert tag refs
    for tag in &refs.tags {
        ensure_tag_exists(tx, tag)?;
        tx.execute(
            "INSERT OR IGNORE INTO entity_refs (source_kind, source_id, target_kind, target_id) VALUES (?1, ?2, 'tag', ?3)",
            params![source_kind, source_id, tag],
        )?;
    }

    // Insert task refs
    for task_id in &refs.tasks {
        tx.execute(
            "INSERT OR IGNORE INTO entity_refs (source_kind, source_id, target_kind, target_id) VALUES (?1, ?2, 'task', ?3)",
            params![source_kind, source_id, task_id],
        )?;
    }

    // Insert note refs
    for note_id in &refs.notes {
        tx.execute(
            "INSERT OR IGNORE INTO entity_refs (source_kind, source_id, target_kind, target_id) VALUES (?1, ?2, 'note', ?3)",
            params![source_kind, source_id, note_id],
        )?;
    }

    Ok(())
}

fn load_refs(conn: &Connection, source_kind: &str, source_id: &str) -> Refs {
    let mut refs = Refs::default();

    let mut stmt = conn.prepare(
        "SELECT target_kind, target_id FROM entity_refs WHERE source_kind = ?1 AND source_id = ?2"
    ).unwrap();

    let rows = stmt.query_map(params![source_kind, source_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }).unwrap();

    for row in rows {
        if let Ok((kind, id)) = row {
            match kind.as_str() {
                "person" => refs.people.push(id),
                "tag" => refs.tags.push(id),
                "task" => refs.tasks.push(id),
                "note" => refs.notes.push(id),
                _ => {}
            }
        }
    }
    refs
}

fn update_fts(tx: &Transaction, entity_kind: &str, entity_id: &str, content: &str) -> Result<()> {
    // Delete existing FTS entry
    tx.execute(
        "DELETE FROM fts_entities WHERE entity_id = ?1 AND entity_kind = ?2",
        params![entity_id, entity_kind],
    )?;
    // Insert new
    tx.execute(
        "INSERT INTO fts_entities (entity_id, entity_kind, content) VALUES (?1, ?2, ?3)",
        params![entity_id, entity_kind, content],
    )?;
    Ok(())
}

fn parse_rfc3339(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn parse_opt_date(s: &Option<String>) -> Option<NaiveDate> {
    s.as_ref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
}

// ── Store implementation ───────────────────────────────────────────

impl Store for SqliteStore {
    // ── Tasks ────────────────────────────────────────────────────

    fn get_task(&self, id: &str) -> Result<Task> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, description, status, created_at, updated_at, due_date, due_time, priority, private, pinned, archived, created_dir FROM tasks WHERE id = ?1"
        )?;

        let task = stmt.query_row(params![id], |row| {
            Ok(Task {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get::<_, String>(2)?,
                status: {
                    let s: String = row.get(3)?;
                    TaskStatus::from_str_loose(&s).unwrap_or(TaskStatus::Backlog)
                },
                created_at: parse_rfc3339(&row.get::<_, String>(4)?),
                updated_at: parse_rfc3339(&row.get::<_, String>(5)?),
                due_date: parse_opt_date(&row.get::<_, Option<String>>(6)?),
                due_time: row.get(7)?,
                priority: {
                    let s: String = row.get(8)?;
                    match s.to_lowercase().as_str() {
                        "low" => Priority::Low,
                        "medium" | "med" => Priority::Medium,
                        "high" => Priority::High,
                        _ => Priority::None,
                    }
                },
                private: row.get::<_, i32>(9)? != 0,
                pinned: row.get::<_, i32>(10)? != 0,
                archived: row.get::<_, i32>(11)? != 0,
                created_dir: row.get(12)?,
                refs: Refs::default(),
                status_history: Vec::new(),
            })
        }).with_context(|| format!("task not found: {}", id))?;

        // Load refs
        let refs = load_refs(&conn, "task", id);
        let mut task = task;
        task.refs = refs;

        // Load status history
        let mut hist_stmt = conn.prepare(
            "SELECT status, at FROM task_status_history WHERE task_id = ?1 ORDER BY at"
        )?;
        let history = hist_stmt.query_map(params![id], |row| {
            let status_str: String = row.get(0)?;
            let at_str: String = row.get(1)?;
            Ok(crate::domain::StatusChange {
                status: TaskStatus::from_str_loose(&status_str).unwrap_or(TaskStatus::Backlog),
                at: parse_rfc3339(&at_str),
            })
        })?.filter_map(|r| r.ok()).collect();
        task.status_history = history;

        Ok(task)
    }

    fn list_tasks(&self) -> Result<Vec<Task>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, description, status, created_at, updated_at, due_date, due_time, priority, private, pinned, archived, created_dir FROM tasks ORDER BY created_at DESC"
        )?;

        let tasks: Vec<Task> = stmt.query_map([], |row| {
            Ok(Task {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get::<_, String>(2)?,
                status: {
                    let s: String = row.get(3)?;
                    TaskStatus::from_str_loose(&s).unwrap_or(TaskStatus::Backlog)
                },
                created_at: parse_rfc3339(&row.get::<_, String>(4)?),
                updated_at: parse_rfc3339(&row.get::<_, String>(5)?),
                due_date: parse_opt_date(&row.get::<_, Option<String>>(6)?),
                due_time: row.get(7)?,
                priority: {
                    let s: String = row.get(8)?;
                    match s.to_lowercase().as_str() {
                        "low" => Priority::Low,
                        "medium" | "med" => Priority::Medium,
                        "high" => Priority::High,
                        _ => Priority::None,
                    }
                },
                private: row.get::<_, i32>(9)? != 0,
                pinned: row.get::<_, i32>(10)? != 0,
                archived: row.get::<_, i32>(11)? != 0,
                created_dir: row.get(12)?,
                refs: Refs::default(),
                status_history: Vec::new(),
            })
        })?.filter_map(|r| r.ok()).collect();

        // Load refs for each task
        let tasks: Vec<Task> = tasks.into_iter().map(|mut t| {
            t.refs = load_refs(&conn, "task", &t.id);
            t
        }).collect();

        Ok(tasks)
    }

    fn save_task(&self, task: &Task) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        tx.execute(
            "INSERT OR REPLACE INTO tasks (id, title, description, status, created_at, updated_at, due_date, due_time, priority, private, pinned, archived, created_dir)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                task.id,
                task.title,
                task.description,
                task.status.as_str(),
                task.created_at.to_rfc3339(),
                task.updated_at.to_rfc3339(),
                task.due_date.map(|d| d.format("%Y-%m-%d").to_string()),
                task.due_time,
                task.priority.label(),
                task.private as i32,
                task.pinned as i32,
                task.archived as i32,
                task.created_dir,
            ],
        )?;

        // Status history
        tx.execute("DELETE FROM task_status_history WHERE task_id = ?1", params![task.id])?;
        for sc in &task.status_history {
            tx.execute(
                "INSERT INTO task_status_history (task_id, status, at) VALUES (?1, ?2, ?3)",
                params![task.id, sc.status.as_str(), sc.at.to_rfc3339()],
            )?;
        }

        // Entity refs — merge @mentions/#tags from title + description
        let mut refs = task.refs.clone();
        merge_text_refs(&mut refs, &format!("{} {}", task.title, task.description));
        save_entity_refs(&tx, "task", &task.id, &refs)?;

        // FTS
        let content = format!("{} {}", task.title, task.description);
        update_fts(&tx, "task", &task.id, &content)?;

        tx.commit()?;
        Ok(())
    }

    fn delete_task(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
        conn.execute("DELETE FROM entity_refs WHERE source_kind = 'task' AND source_id = ?1", params![id])?;
        conn.execute("DELETE FROM fts_entities WHERE entity_id = ?1 AND entity_kind = 'task'", params![id])?;
        Ok(())
    }

    // ── Notes ────────────────────────────────────────────────────

    fn get_note(&self, id: &str) -> Result<Note> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, created_at, updated_at, private, pinned, archived, created_dir, body FROM notes WHERE id = ?1"
        )?;

        let note = stmt.query_row(params![id], |row| {
            Ok(Note {
                id: row.get(0)?,
                title: row.get(1)?,
                created_at: parse_rfc3339(&row.get::<_, String>(2)?),
                updated_at: parse_rfc3339(&row.get::<_, String>(3)?),
                private: row.get::<_, i32>(4)? != 0,
                pinned: row.get::<_, i32>(5)? != 0,
                archived: row.get::<_, i32>(6)? != 0,
                created_dir: row.get(7)?,
                refs: Refs::default(),
                body: row.get(8)?,
            })
        }).with_context(|| format!("note not found: {}", id))?;

        let refs = load_refs(&conn, "note", id);
        let mut note = note;
        note.refs = refs;
        Ok(note)
    }

    fn list_notes(&self) -> Result<Vec<Note>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, created_at, updated_at, private, pinned, archived, created_dir, body FROM notes ORDER BY created_at DESC"
        )?;

        let notes: Vec<Note> = stmt.query_map([], |row| {
            Ok(Note {
                id: row.get(0)?,
                title: row.get(1)?,
                created_at: parse_rfc3339(&row.get::<_, String>(2)?),
                updated_at: parse_rfc3339(&row.get::<_, String>(3)?),
                private: row.get::<_, i32>(4)? != 0,
                pinned: row.get::<_, i32>(5)? != 0,
                archived: row.get::<_, i32>(6)? != 0,
                created_dir: row.get(7)?,
                refs: Refs::default(),
                body: row.get(8)?,
            })
        })?.filter_map(|r| r.ok()).collect();

        let notes: Vec<Note> = notes.into_iter().map(|mut n| {
            n.refs = load_refs(&conn, "note", &n.id);
            n
        }).collect();

        Ok(notes)
    }

    fn save_note(&self, note: &Note) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        tx.execute(
            "INSERT OR REPLACE INTO notes (id, title, created_at, updated_at, private, pinned, archived, created_dir, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                note.id,
                note.title,
                note.created_at.to_rfc3339(),
                note.updated_at.to_rfc3339(),
                note.private as i32,
                note.pinned as i32,
                note.archived as i32,
                note.created_dir,
                note.body,
            ],
        )?;

        // Entity refs — merge @mentions/#tags from title + body
        let mut refs = note.refs.clone();
        merge_text_refs(&mut refs, &format!("{} {}", note.title, note.body));
        save_entity_refs(&tx, "note", &note.id, &refs)?;

        let content = format!("{} {}", note.title, note.body);
        update_fts(&tx, "note", &note.id, &content)?;

        tx.commit()?;
        Ok(())
    }

    fn delete_note(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM notes WHERE id = ?1", params![id])?;
        conn.execute("DELETE FROM entity_refs WHERE source_kind = 'note' AND source_id = ?1", params![id])?;
        conn.execute("DELETE FROM fts_entities WHERE entity_id = ?1 AND entity_kind = 'note'", params![id])?;
        Ok(())
    }

    // ── People ───────────────────────────────────────────────────

    fn get_person(&self, slug: &str) -> Result<Person> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT slug, created_at, pinned, archived FROM persons WHERE slug = ?1"
        )?;

        let person = stmt.query_row(params![slug], |row| {
            Ok(Person {
                slug: row.get(0)?,
                created_at: parse_rfc3339(&row.get::<_, String>(1)?),
                pinned: row.get::<_, i32>(2)? != 0,
                archived: row.get::<_, i32>(3)? != 0,
                metadata: std::collections::HashMap::new(),
            })
        }).with_context(|| format!("person not found: {}", slug))?;

        // Load metadata
        let mut meta_stmt = conn.prepare(
            "SELECT key, value FROM person_metadata WHERE person_slug = ?1"
        )?;
        let mut person = person;
        let metadata: std::collections::HashMap<String, String> = meta_stmt
            .query_map(params![slug], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        person.metadata = metadata;

        Ok(person)
    }

    fn list_persons(&self) -> Result<Vec<Person>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT slug, created_at, pinned, archived FROM persons ORDER BY slug"
        )?;

        let persons: Vec<Person> = stmt.query_map([], |row| {
            Ok(Person {
                slug: row.get(0)?,
                created_at: parse_rfc3339(&row.get::<_, String>(1)?),
                pinned: row.get::<_, i32>(2)? != 0,
                archived: row.get::<_, i32>(3)? != 0,
                metadata: std::collections::HashMap::new(),
            })
        })?.filter_map(|r| r.ok()).collect();

        // Load metadata for each person
        let persons: Vec<Person> = persons.into_iter().map(|mut p| {
            let mut meta_stmt = conn.prepare(
                "SELECT key, value FROM person_metadata WHERE person_slug = ?1"
            ).unwrap();
            p.metadata = meta_stmt
                .query_map(params![p.slug], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                }).unwrap()
                .filter_map(|r| r.ok())
                .collect();
            p
        }).collect();

        Ok(persons)
    }

    fn save_person(&self, person: &Person) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        tx.execute(
            "INSERT OR REPLACE INTO persons (slug, created_at, pinned, archived)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                person.slug,
                person.created_at.to_rfc3339(),
                person.pinned as i32,
                person.archived as i32,
            ],
        )?;

        // Update metadata
        tx.execute("DELETE FROM person_metadata WHERE person_slug = ?1", params![person.slug])?;
        for (key, value) in &person.metadata {
            tx.execute(
                "INSERT INTO person_metadata (person_slug, key, value) VALUES (?1, ?2, ?3)",
                params![person.slug, key, value],
            )?;
        }

        // FTS
        let meta_text: String = person.metadata.values().cloned().collect::<Vec<_>>().join(" ");
        let content = format!("{} {}", person.slug, meta_text);
        update_fts(&tx, "person", &person.slug, &content)?;

        tx.commit()?;
        Ok(())
    }

    fn delete_person(&self, slug: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM persons WHERE slug = ?1", params![slug])?;
        conn.execute("DELETE FROM fts_entities WHERE entity_id = ?1 AND entity_kind = 'person'", params![slug])?;
        Ok(())
    }

    // ── Tags ─────────────────────────────────────────────────────

    fn get_tag(&self, slug: &str) -> Result<Tag> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT slug, created_at FROM tags WHERE slug = ?1"
        )?;

        stmt.query_row(params![slug], |row| {
            Ok(Tag {
                slug: row.get(0)?,
                created_at: parse_rfc3339(&row.get::<_, String>(1)?),
            })
        }).with_context(|| format!("tag not found: {}", slug))
    }

    fn list_tags(&self) -> Result<Vec<Tag>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT slug, created_at FROM tags ORDER BY slug")?;

        let tags: Vec<Tag> = stmt.query_map([], |row| {
            Ok(Tag {
                slug: row.get(0)?,
                created_at: parse_rfc3339(&row.get::<_, String>(1)?),
            })
        })?.filter_map(|r| r.ok()).collect();

        Ok(tags)
    }

    fn save_tag(&self, tag: &Tag) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO tags (slug, created_at) VALUES (?1, ?2)",
            params![tag.slug, tag.created_at.to_rfc3339()],
        )?;
        update_fts(&tx, "tag", &tag.slug, &tag.slug)?;
        tx.commit()?;
        Ok(())
    }

    fn delete_tag(&self, slug: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM tags WHERE slug = ?1", params![slug])?;
        conn.execute("DELETE FROM fts_entities WHERE entity_id = ?1 AND entity_kind = 'tag'", params![slug])?;
        Ok(())
    }

    // ── Agendas ─────────────────────────────────────────────────

    fn get_agenda(&self, id: &str) -> Result<Agenda> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, person_slug, date, created_at, updated_at, body FROM agendas WHERE id = ?1"
        )?;

        let agenda = stmt.query_row(params![id], |row| {
            let date_str: String = row.get(3)?;
            Ok(Agenda {
                id: row.get(0)?,
                title: row.get(1)?,
                person_slug: row.get(2)?,
                date: NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").unwrap_or_else(|_| NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()),
                created_at: parse_rfc3339(&row.get::<_, String>(4)?),
                updated_at: parse_rfc3339(&row.get::<_, String>(5)?),
                body: row.get(6)?,
                refs: Refs::default(),
            })
        }).with_context(|| format!("agenda not found: {}", id))?;

        let refs = load_refs(&conn, "agenda", id);
        let mut agenda = agenda;
        agenda.refs = refs;
        Ok(agenda)
    }

    fn list_agendas(&self) -> Result<Vec<Agenda>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, person_slug, date, created_at, updated_at, body FROM agendas ORDER BY date DESC"
        )?;

        let agendas: Vec<Agenda> = stmt.query_map([], |row| {
            let date_str: String = row.get(3)?;
            Ok(Agenda {
                id: row.get(0)?,
                title: row.get(1)?,
                person_slug: row.get(2)?,
                date: NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").unwrap_or_else(|_| NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()),
                created_at: parse_rfc3339(&row.get::<_, String>(4)?),
                updated_at: parse_rfc3339(&row.get::<_, String>(5)?),
                body: row.get(6)?,
                refs: Refs::default(),
            })
        })?.filter_map(|r| r.ok()).collect();

        let agendas: Vec<Agenda> = agendas.into_iter().map(|mut a| {
            a.refs = load_refs(&conn, "agenda", &a.id);
            a
        }).collect();

        Ok(agendas)
    }

    fn list_agendas_for_person(&self, person_slug: &str) -> Result<Vec<Agenda>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, person_slug, date, created_at, updated_at, body FROM agendas WHERE person_slug = ?1 ORDER BY date DESC"
        )?;

        let agendas: Vec<Agenda> = stmt.query_map(params![person_slug], |row| {
            let date_str: String = row.get(3)?;
            Ok(Agenda {
                id: row.get(0)?,
                title: row.get(1)?,
                person_slug: row.get(2)?,
                date: NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").unwrap_or_else(|_| NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()),
                created_at: parse_rfc3339(&row.get::<_, String>(4)?),
                updated_at: parse_rfc3339(&row.get::<_, String>(5)?),
                body: row.get(6)?,
                refs: Refs::default(),
            })
        })?.filter_map(|r| r.ok()).collect();

        let agendas: Vec<Agenda> = agendas.into_iter().map(|mut a| {
            a.refs = load_refs(&conn, "agenda", &a.id);
            a
        }).collect();

        Ok(agendas)
    }

    fn save_agenda(&self, agenda: &Agenda) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        // Ensure the person exists
        ensure_person_exists(&tx, &agenda.person_slug)?;

        tx.execute(
            "INSERT OR REPLACE INTO agendas (id, title, person_slug, date, created_at, updated_at, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                agenda.id,
                agenda.title,
                agenda.person_slug,
                agenda.date.format("%Y-%m-%d").to_string(),
                agenda.created_at.to_rfc3339(),
                agenda.updated_at.to_rfc3339(),
                agenda.body,
            ],
        )?;

        // Entity refs — merge @mentions/#tags from title + body
        let mut refs = agenda.refs.clone();
        merge_text_refs(&mut refs, &format!("{} {}", agenda.title, agenda.body));
        save_entity_refs(&tx, "agenda", &agenda.id, &refs)?;

        let content = format!("{} {}", agenda.title, agenda.body);
        update_fts(&tx, "agenda", &agenda.id, &content)?;

        tx.commit()?;
        Ok(())
    }

    fn delete_agenda(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM agendas WHERE id = ?1", params![id])?;
        conn.execute("DELETE FROM entity_refs WHERE source_kind = 'agenda' AND source_id = ?1", params![id])?;
        conn.execute("DELETE FROM fts_entities WHERE entity_id = ?1 AND entity_kind = 'agenda'", params![id])?;
        Ok(())
    }

    // ── Query ────────────────────────────────────────────────────

    fn rebuild_index(&self) -> Result<()> {
        // No-op for SQLite — indexes are maintained transactionally.
        Ok(())
    }

    fn entities_by_date(&self, date: &str) -> Vec<EntityRef> {
        let conn = self.conn.lock().unwrap();
        let mut results = Vec::new();

        // Tasks with due_date or created_at matching
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id FROM tasks WHERE due_date = ?1 OR substr(created_at, 1, 10) = ?1"
        ) {
            if let Ok(rows) = stmt.query_map(params![date], |row| row.get::<_, String>(0)) {
                for id in rows.flatten() {
                    results.push(EntityRef { kind: EntityKind::Task, id });
                }
            }
        }

        // Notes with created_at matching
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id FROM notes WHERE substr(created_at, 1, 10) = ?1"
        ) {
            if let Ok(rows) = stmt.query_map(params![date], |row| row.get::<_, String>(0)) {
                for id in rows.flatten() {
                    results.push(EntityRef { kind: EntityKind::Note, id });
                }
            }
        }

        // Agendas with matching date
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id FROM agendas WHERE date = ?1"
        ) {
            if let Ok(rows) = stmt.query_map(params![date], |row| row.get::<_, String>(0)) {
                for id in rows.flatten() {
                    results.push(EntityRef { kind: EntityKind::Agenda, id });
                }
            }
        }

        results
    }

    fn get_memory(&self, slug: &str) -> Vec<EntityRef> {
        let conn = self.conn.lock().unwrap();
        let mut results = Vec::new();

        // Find all entities that reference this person or tag
        if let Ok(mut stmt) = conn.prepare(
            "SELECT source_kind, source_id FROM entity_refs WHERE (target_kind = 'person' AND target_id = ?1) OR (target_kind = 'tag' AND target_id = ?1)"
        ) {
            if let Ok(rows) = stmt.query_map(params![slug], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
                for row in rows.flatten() {
                    let (kind_str, id) = row;
                    let kind = match kind_str.as_str() {
                        "task" => EntityKind::Task,
                        "note" => EntityKind::Note,
                        "agenda" => EntityKind::Agenda,
                        _ => continue,
                    };
                    results.push(EntityRef { kind, id });
                }
            }
        }

        // Also include agendas for this person
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id FROM agendas WHERE person_slug = ?1"
        ) {
            if let Ok(rows) = stmt.query_map(params![slug], |row| row.get::<_, String>(0)) {
                for id in rows.flatten() {
                    if !results.iter().any(|r| r.id == id) {
                        results.push(EntityRef { kind: EntityKind::Agenda, id });
                    }
                }
            }
        }

        results
    }

    fn search(&self, query: &str) -> Vec<EntityRef> {
        let conn = self.conn.lock().unwrap();
        let mut results = Vec::new();

        // Try FTS5 MATCH first
        let fts_query = format!("{}*", query.replace('"', ""));
        if let Ok(mut stmt) = conn.prepare(
            "SELECT entity_id, entity_kind FROM fts_entities WHERE content MATCH ?1 ORDER BY rank LIMIT 50"
        ) {
            if let Ok(rows) = stmt.query_map(params![fts_query], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
                for row in rows.flatten() {
                    let (id, kind_str) = row;
                    let kind = match kind_str.as_str() {
                        "task" => EntityKind::Task,
                        "note" => EntityKind::Note,
                        "person" => EntityKind::Person,
                        "tag" => EntityKind::Tag,
                        "agenda" => EntityKind::Agenda,
                        _ => continue,
                    };
                    results.push(EntityRef { kind, id });
                }
            }
        }

        results
    }

    fn person_frecency_scores(&self) -> std::collections::HashMap<String, f64> {
        let conn = self.conn.lock().unwrap();
        let mut scores = std::collections::HashMap::new();

        // Union three sources of timeline items linked to people:
        //   tasks and notes via entity_refs, agendas via person_slug.
        // For each item compute 1 / (1 + age_days / 30) and sum per person.
        let sql = "
            SELECT person_slug,
                   SUM(1.0 / (1.0 + (julianday('now') - julianday(item_date)) / 30.0)) AS score
            FROM (
                SELECT er.target_id AS person_slug, t.updated_at AS item_date
                FROM entity_refs er
                JOIN tasks t ON er.source_kind = 'task' AND er.source_id = t.id
                WHERE er.target_kind = 'person'

                UNION ALL

                SELECT er.target_id AS person_slug, n.updated_at AS item_date
                FROM entity_refs er
                JOIN notes n ON er.source_kind = 'note' AND er.source_id = n.id
                WHERE er.target_kind = 'person'

                UNION ALL

                SELECT a.person_slug, a.date AS item_date
                FROM agendas a
            )
            GROUP BY person_slug
        ";

        if let Ok(mut stmt) = conn.prepare(sql) {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            }) {
                for row in rows.flatten() {
                    scores.insert(row.0, row.1);
                }
            }
        }

        scores
    }

    fn rename_person(&self, old_slug: &str, new_slug: &str) -> Result<()> {
        anyhow::ensure!(old_slug != new_slug, "new slug is the same as the old one");

        let mut conn = self.conn.lock().unwrap();

        // Reject if new_slug already exists.
        let exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM persons WHERE slug = ?1",
            params![new_slug],
            |row| row.get::<_, i64>(0),
        )? > 0;
        anyhow::ensure!(!exists, "a person with slug @{} already exists", new_slug);

        // Collect tasks whose title or description mention the old slug.
        let like = format!("%@{}%", old_slug);
        let tasks: Vec<(String, String, String)> = {
            let mut stmt = conn.prepare(
                "SELECT id, title, description FROM tasks WHERE title LIKE ?1 OR description LIKE ?1",
            )?;
            stmt.query_map(params![like], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?.flatten().collect()
        };

        // Collect notes whose title or body mention the old slug.
        let notes: Vec<(String, String, String)> = {
            let mut stmt = conn.prepare(
                "SELECT id, title, body FROM notes WHERE title LIKE ?1 OR body LIKE ?1",
            )?;
            stmt.query_map(params![like], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?.flatten().collect()
        };

        // Collect agendas that either belong to this person or mention the slug in text.
        let agendas: Vec<(String, String, String)> = {
            let mut stmt = conn.prepare(
                "SELECT id, title, body FROM agendas WHERE person_slug = ?1 OR title LIKE ?2 OR body LIKE ?2",
            )?;
            stmt.query_map(params![old_slug, like], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?.flatten().collect()
        };

        let tx = conn.transaction()?;

        // 1. Rename the persons row — cascades to person_metadata and agendas.person_slug
        //    via ON UPDATE CASCADE (requires foreign_keys = ON, set at connection open).
        tx.execute("UPDATE persons SET slug = ?1 WHERE slug = ?2", params![new_slug, old_slug])?;

        // 2. Rewrite entity_refs (no FK on this table).
        tx.execute(
            "UPDATE entity_refs SET target_id = ?1 WHERE target_kind = 'person' AND target_id = ?2",
            params![new_slug, old_slug],
        )?;

        // 3. Rebuild FTS entry for the person.
        tx.execute(
            "DELETE FROM fts_entities WHERE entity_kind = 'person' AND entity_id = ?1",
            params![old_slug],
        )?;
        let meta_text: String = {
            let mut stmt = tx.prepare("SELECT value FROM person_metadata WHERE person_slug = ?1")?;
            stmt.query_map(params![new_slug], |row| row.get::<_, String>(0))?
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
        };
        tx.execute(
            "INSERT INTO fts_entities (entity_id, entity_kind, content) VALUES (?1, 'person', ?2)",
            params![new_slug, format!("{} {}", new_slug, meta_text)],
        )?;

        // 4. Rewrite @old_slug in task text and update FTS.
        for (id, title, desc) in tasks {
            let new_title = replace_mention(&title, old_slug, new_slug);
            let new_desc  = replace_mention(&desc,  old_slug, new_slug);
            if new_title != title || new_desc != desc {
                tx.execute(
                    "UPDATE tasks SET title = ?1, description = ?2 WHERE id = ?3",
                    params![new_title, new_desc, id],
                )?;
                tx.execute("DELETE FROM fts_entities WHERE entity_id = ?1 AND entity_kind = 'task'", params![id])?;
                tx.execute(
                    "INSERT INTO fts_entities (entity_id, entity_kind, content) VALUES (?1, 'task', ?2)",
                    params![id, format!("{} {}", new_title, new_desc)],
                )?;
            }
        }

        // 5. Rewrite @old_slug in note text and update FTS.
        for (id, title, body) in notes {
            let new_title = replace_mention(&title, old_slug, new_slug);
            let new_body  = replace_mention(&body,  old_slug, new_slug);
            if new_title != title || new_body != body {
                tx.execute(
                    "UPDATE notes SET title = ?1, body = ?2 WHERE id = ?3",
                    params![new_title, new_body, id],
                )?;
                tx.execute("DELETE FROM fts_entities WHERE entity_id = ?1 AND entity_kind = 'note'", params![id])?;
                tx.execute(
                    "INSERT INTO fts_entities (entity_id, entity_kind, content) VALUES (?1, 'note', ?2)",
                    params![id, format!("{} {}", new_title, new_body)],
                )?;
            }
        }

        // 6. Rewrite @old_slug in agenda text and update FTS.
        for (id, title, body) in agendas {
            let new_title = replace_mention(&title, old_slug, new_slug);
            let new_body  = replace_mention(&body,  old_slug, new_slug);
            if new_title != title || new_body != body {
                tx.execute(
                    "UPDATE agendas SET title = ?1, body = ?2 WHERE id = ?3",
                    params![new_title, new_body, id],
                )?;
                tx.execute("DELETE FROM fts_entities WHERE entity_id = ?1 AND entity_kind = 'agenda'", params![id])?;
                tx.execute(
                    "INSERT INTO fts_entities (entity_id, entity_kind, content) VALUES (?1, 'agenda', ?2)",
                    params![id, format!("{} {}", new_title, new_body)],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }
}
