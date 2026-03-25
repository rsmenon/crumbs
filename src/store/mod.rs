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

}
