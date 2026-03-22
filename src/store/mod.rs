pub mod flatfile;
pub mod index;
pub mod fts;
pub mod io;

pub use flatfile::FlatFileStore;

use anyhow::Result;
use crate::domain::*;

/// Core persistence trait.
///
/// All data access goes through this trait so that views and the app
/// are decoupled from the concrete file-system layout.
#[allow(unused)]
pub trait Store: Send + Sync {
    // ── Tasks ────────────────────────────────────────────────────

    fn get_task(&self, id: &str) -> Result<Task>;
    fn list_tasks(&self) -> Result<Vec<Task>>;
    fn save_task(&self, task: &Task) -> Result<()>;
    fn delete_task(&self, id: &str) -> Result<()>;

    // ── Todos ────────────────────────────────────────────────────

    fn get_todo(&self, id: &str) -> Result<Todo>;
    fn list_todos(&self) -> Result<Vec<Todo>>;
    fn save_todo(&self, todo: &Todo) -> Result<()>;
    fn delete_todo(&self, id: &str) -> Result<()>;

    // ── Reminders ────────────────────────────────────────────────

    fn get_reminder(&self, id: &str) -> Result<Reminder>;
    fn list_reminders(&self) -> Result<Vec<Reminder>>;
    fn save_reminder(&self, reminder: &Reminder) -> Result<()>;
    fn delete_reminder(&self, id: &str) -> Result<()>;

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

    // ── Topics ───────────────────────────────────────────────────

    fn get_topic(&self, slug: &str) -> Result<Topic>;
    fn list_topics(&self) -> Result<Vec<Topic>>;
    fn save_topic(&self, topic: &Topic) -> Result<()>;
    fn delete_topic(&self, slug: &str) -> Result<()>;

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
