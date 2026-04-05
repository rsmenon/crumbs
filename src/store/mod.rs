pub mod io;
pub mod sqlite;

pub use sqlite::SqliteStore;

use anyhow::Result;
use crate::domain::*;

/// Core persistence trait.
///
/// All data access goes through this trait so that views and the app
/// are decoupled from the concrete storage implementation.
#[allow(unused)]
pub trait Store: Send + Sync {
    // ── Tasks ────────────────────────────────────────────────────

    fn get_task(&self, id: &str) -> Result<Task>;
    fn list_tasks(&self) -> Result<Vec<Task>>;
    fn save_task(&self, task: &Task) -> Result<()>;
    fn delete_task(&self, id: &str) -> Result<()>;

    // ── Notes ────────────────────────────────────────────────────

    fn get_note(&self, id: &str) -> Result<Note>;
    fn list_notes(&self) -> Result<Vec<Note>>;
    fn save_note(&self, note: &Note) -> Result<()>;
    fn delete_note(&self, id: &str) -> Result<()>;

    // ── People ───────────────────────────────────────────────────

    fn get_person(&self, slug: &str) -> Result<Person>;
    fn list_persons(&self) -> Result<Vec<Person>>;
    fn save_person(&self, person: &Person) -> Result<()>;
    fn delete_person(&self, slug: &str) -> Result<()>;
    /// Rename a person's slug atomically: updates the persons row, cascades to
    /// person_metadata and agendas via FK, rewrites entity_refs and FTS, and
    /// replaces @old_slug with @new_slug in task/note/agenda text fields.
    /// Returns an error if `new_slug` already exists.
    fn rename_person(&self, old_slug: &str, new_slug: &str) -> Result<()>;

    // ── Tags ─────────────────────────────────────────────────────

    fn get_tag(&self, slug: &str) -> Result<Tag>;
    fn list_tags(&self) -> Result<Vec<Tag>>;
    fn save_tag(&self, tag: &Tag) -> Result<()>;
    fn delete_tag(&self, slug: &str) -> Result<()>;

    // ── Agendas ─────────────────────────────────────────────────

    fn get_agenda(&self, id: &str) -> Result<Agenda>;
    fn list_agendas(&self) -> Result<Vec<Agenda>>;
    fn list_agendas_for_person(&self, person_slug: &str) -> Result<Vec<Agenda>>;
    fn save_agenda(&self, agenda: &Agenda) -> Result<()>;
    fn delete_agenda(&self, id: &str) -> Result<()>;

    // ── Query ────────────────────────────────────────────────────

    fn rebuild_index(&self) -> Result<()>;
    fn entities_by_date(&self, date: &str) -> Vec<EntityRef>;
    fn get_memory(&self, slug: &str) -> Vec<EntityRef>;
    fn search(&self, query: &str) -> Vec<EntityRef>;

    /// Frecency scores for all people.
    ///
    /// Each timeline item (task, note, agenda) linked to a person contributes
    /// `1 / (1 + age_days / 30)` to that person's score — full weight today,
    /// half weight at 30 days, falling off with a 30-day half-life.
    /// Returns a map of person slug → score (missing slugs score 0).
    fn person_frecency_scores(&self) -> std::collections::HashMap<String, f64>;

    // ── Cross-references ─────────────────────────────────────────

    /// Add an explicit cross-reference between two entities.
    /// Uses INSERT OR IGNORE so calling it twice is safe.
    fn add_entity_ref(&self, src_kind: &str, src_id: &str, tgt_kind: &str, tgt_id: &str) -> Result<()>;

    /// Remove an explicit cross-reference between two entities.
    fn remove_entity_ref(&self, src_kind: &str, src_id: &str, tgt_kind: &str, tgt_id: &str) -> Result<()>;

    /// Return all entities that reference the given entity (reverse lookup).
    fn get_backlinks(&self, target_kind: &str, target_id: &str) -> Vec<EntityRef>;

}
