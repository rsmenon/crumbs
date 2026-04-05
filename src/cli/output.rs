use std::collections::HashMap;

use serde::Serialize;

use crate::domain::{Agenda, Note, Person, Refs, Tag, Task};

#[derive(Serialize)]
pub struct TaskOutput {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: crate::domain::TaskStatus,
    pub priority: crate::domain::Priority,
    pub due_date: Option<String>,
    pub due_time: Option<String>,
    pub private: bool,
    pub pinned: bool,
    pub archived: bool,
    pub refs: Refs,
    pub created_at: String,
    pub updated_at: String,
}

impl From<Task> for TaskOutput {
    fn from(t: Task) -> Self {
        Self {
            id: t.id,
            title: t.title,
            description: t.description,
            status: t.status,
            priority: t.priority,
            due_date: t.due_date.map(|d| d.format("%Y-%m-%d").to_string()),
            due_time: t.due_time,
            private: t.private,
            pinned: t.pinned,
            archived: t.archived,
            refs: t.refs,
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Serialize)]
pub struct NoteOutput {
    pub id: String,
    pub title: String,
    pub body: String,
    pub private: bool,
    pub pinned: bool,
    pub archived: bool,
    pub refs: Refs,
    pub created_at: String,
    pub updated_at: String,
}

impl From<Note> for NoteOutput {
    fn from(n: Note) -> Self {
        Self {
            id: n.id,
            title: n.title,
            body: n.body,
            private: n.private,
            pinned: n.pinned,
            archived: n.archived,
            refs: n.refs,
            created_at: n.created_at.to_rfc3339(),
            updated_at: n.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Serialize)]
pub struct AgendaOutput {
    pub id: String,
    pub title: String,
    pub person_slug: String,
    pub date: String,
    pub body: String,
    pub refs: Refs,
    pub created_at: String,
    pub updated_at: String,
}

impl From<Agenda> for AgendaOutput {
    fn from(a: Agenda) -> Self {
        Self {
            id: a.id,
            title: a.title,
            person_slug: a.person_slug,
            date: a.date.format("%Y-%m-%d").to_string(),
            body: a.body,
            refs: a.refs,
            created_at: a.created_at.to_rfc3339(),
            updated_at: a.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Serialize)]
pub struct PersonOutput {
    pub slug: String,
    pub pinned: bool,
    pub archived: bool,
    pub metadata: HashMap<String, String>,
    pub created_at: String,
}

impl From<Person> for PersonOutput {
    fn from(p: Person) -> Self {
        Self {
            slug: p.slug,
            pinned: p.pinned,
            archived: p.archived,
            metadata: p.metadata,
            created_at: p.created_at.to_rfc3339(),
        }
    }
}

#[derive(Serialize)]
pub struct TagOutput {
    pub slug: String,
    pub created_at: String,
}

impl From<Tag> for TagOutput {
    fn from(t: Tag) -> Self {
        Self {
            slug: t.slug,
            created_at: t.created_at.to_rfc3339(),
        }
    }
}

#[derive(Serialize)]
pub struct SearchResult {
    pub kind: String,
    pub id: String,
    pub title: String,
}

#[derive(Serialize)]
pub struct DeleteResult {
    pub deleted: String,
}

#[derive(Serialize)]
pub struct LinkEntry {
    pub kind: String,
    pub id: String,
    pub title: String,
}

#[derive(Serialize)]
pub struct LinksOutput {
    pub linked: Vec<LinkEntry>,
    pub backlinks: Vec<LinkEntry>,
}

#[derive(Serialize)]
pub struct LinkActionResult {
    pub source_kind: String,
    pub source_id: String,
    pub target_kind: String,
    pub target_id: String,
    pub action: String,
}

pub fn print_json<T: serde::Serialize>(out: &mut dyn std::io::Write, value: &T) -> anyhow::Result<()> {
    writeln!(out, "{}", serde_json::to_string_pretty(value)?)?;
    Ok(())
}

pub fn print_error(msg: &str) {
    let e = serde_json::json!({"error": msg});
    eprintln!("{}", serde_json::to_string_pretty(&e).unwrap());
}
