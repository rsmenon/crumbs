use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::{Context, Result};

use crate::domain::*;
use crate::parser;

use super::index::Index;
use super::io;
use super::Store;

/// Flat-file store backed by JSON, markdown, and JSONL files on disk.
///
/// Directory layout under `data_dir`:
/// ```text
/// tasks/{id}.json
/// todos/{id}.json
/// reminders/{id}.json
/// notes/{id}.md
/// sink/{YYYY-MM-DD}.jsonl
/// people/{slug}.json
/// topics/{slug}.json
/// ```
pub struct FlatFileStore {
    data_dir: PathBuf,
    index: RwLock<Index>,
}

impl FlatFileStore {
    /// Create a new store rooted at `data_dir`.
    ///
    /// Creates all subdirectories if they do not already exist.
    /// Initialises a git manager if the data directory is (or can be) a git repo.
    pub fn new(data_dir: &Path) -> Result<Self> {
        let subdirs = ["tasks", "todos", "notes", "reminders", "people", "topics", "agendas"];
        for sub in &subdirs {
            fs::create_dir_all(data_dir.join(sub))
                .with_context(|| format!("creating {}/{}", data_dir.display(), sub))?;
        }

        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            index: RwLock::new(Index::new()),
        })
    }

    // ── path helpers ────────────────────────────────────────────

    fn task_path(&self, id: &str) -> PathBuf {
        self.data_dir.join("tasks").join(format!("{}.json", id))
    }

    fn todo_path(&self, id: &str) -> PathBuf {
        self.data_dir.join("todos").join(format!("{}.json", id))
    }

    fn reminder_path(&self, id: &str) -> PathBuf {
        self.data_dir.join("reminders").join(format!("{}.json", id))
    }

    fn note_path(&self, id: &str) -> PathBuf {
        self.data_dir.join("notes").join(format!("{}.md", id))
    }

    fn person_path(&self, slug: &str) -> PathBuf {
        self.data_dir.join("people").join(format!("{}.json", slug))
    }

    fn topic_path(&self, slug: &str) -> PathBuf {
        self.data_dir.join("topics").join(format!("{}.json", slug))
    }

    fn agenda_path(&self, id: &str) -> PathBuf {
        self.data_dir.join("agendas").join(format!("{}.md", id))
    }

    // ── index helpers ───────────────────────────────────────────

    /// Merge explicit refs with text-extracted mentions/topics.
    fn merge_people(refs: &Refs, text: &str) -> Vec<String> {
        let mut people: Vec<String> = refs.people.clone();
        for m in parser::extract_mentions(text) {
            if !people.contains(&m) {
                people.push(m);
            }
        }
        people
    }

    fn merge_topics(refs: &Refs, text: &str) -> Vec<String> {
        let mut topics: Vec<String> = refs.topics.clone();
        for t in parser::extract_topics(text) {
            if !topics.contains(&t) {
                topics.push(t);
            }
        }
        topics
    }

    /// Format a chrono DateTime as YYYY-MM-DD.
    fn format_date(dt: &chrono::DateTime<chrono::Utc>) -> String {
        dt.format("%Y-%m-%d").to_string()
    }

    // ── directory walking helpers ────────────────────────────────

    /// List all files in a subdirectory matching a given extension.
    fn list_dir_files(dir: &Path, ext: &str) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(e) = path.extension() {
                        if e == ext {
                            paths.push(path);
                        }
                    }
                }
            }
        }
        paths
    }

    // ── indexing ─────────────────────────────────────────────────

    fn index_task(index: &mut Index, task: &Task) {
        let mut dates = Vec::new();
        if let Some(due) = task.due_date {
            dates.push(due.format("%Y-%m-%d").to_string());
        }
        dates.push(Self::format_date(&task.created_at));

        let text = format!("{} {}", task.title, task.description);
        let people = Self::merge_people(&task.refs, &text);
        let topics = Self::merge_topics(&task.refs, &text);

        let search_text = format!("{} {}", task.title, task.description);

        let status_str = serde_json::to_value(&task.status)
            .ok()
            .and_then(|v| v.as_str().map(String::from));

        index.add(
            EntityRef {
                kind: EntityKind::Task,
                id: task.id.clone(),
            },
            &task.id,
            &dates,
            &people,
            &topics,
            status_str.as_deref(),
            "task",
            &search_text,
        );
    }

    fn index_todo(index: &mut Index, todo: &Todo) {
        let mut dates = Vec::new();
        if let Some(due) = todo.due_date {
            dates.push(due.format("%Y-%m-%d").to_string());
        }
        dates.push(Self::format_date(&todo.created_at));

        let people = Self::merge_people(&todo.refs, &todo.title);
        let topics = Self::merge_topics(&todo.refs, &todo.title);

        let status = if todo.done { Some("done") } else { Some("pending") };

        index.add(
            EntityRef {
                kind: EntityKind::Todo,
                id: todo.id.clone(),
            },
            &todo.id,
            &dates,
            &people,
            &topics,
            status,
            "todo",
            &todo.title,
        );
    }

    fn index_note(index: &mut Index, note: &Note) {
        let dates = vec![Self::format_date(&note.created_at)];

        let text = format!("{} {}", note.title, note.body);
        let people = Self::merge_people(&note.refs, &text);
        let topics = Self::merge_topics(&note.refs, &text);

        let search_text = format!("{} {}", note.title, note.body);

        index.add(
            EntityRef {
                kind: EntityKind::Note,
                id: note.id.clone(),
            },
            &note.id,
            &dates,
            &people,
            &topics,
            None,
            "note",
            &search_text,
        );
    }

    fn index_reminder(index: &mut Index, reminder: &Reminder) {
        let dates = vec![
            Self::format_date(&reminder.remind_at),
            Self::format_date(&reminder.created_at),
        ];

        let people = Self::merge_people(&reminder.refs, &reminder.title);
        let topics = Self::merge_topics(&reminder.refs, &reminder.title);

        let status = if reminder.dismissed {
            Some("dismissed")
        } else {
            Some("active")
        };

        index.add(
            EntityRef {
                kind: EntityKind::Reminder,
                id: reminder.id.clone(),
            },
            &reminder.id,
            &dates,
            &people,
            &topics,
            status,
            "reminder",
            &reminder.title,
        );
    }

    fn index_person(index: &mut Index, person: &Person) {
        let dates = vec![Self::format_date(&person.created_at)];

        let meta_values: Vec<&str> = person.metadata.values().map(|v| v.as_str()).collect();
        let search_text = format!(
            "{} {} {}",
            person.slug,
            meta_values.join(" "),
            person.tags.join(" ")
        );

        let topics: Vec<String> = person.tags.clone();

        index.add(
            EntityRef {
                kind: EntityKind::Person,
                id: person.slug.clone(),
            },
            &person.slug,
            &dates,
            &[], // people don't reference other people in this context
            &topics,
            None,
            "person",
            &search_text,
        );
    }

    fn index_topic(index: &mut Index, topic: &Topic) {
        let dates = vec![Self::format_date(&topic.created_at)];

        let search_text = format!(
            "{} {} {} {}",
            topic.slug,
            topic.display_name,
            topic.description,
            topic.aliases.join(" ")
        );

        index.add(
            EntityRef {
                kind: EntityKind::Topic,
                id: topic.slug.clone(),
            },
            &topic.slug,
            &dates,
            &[],
            &[], // topics don't self-reference
            None,
            "topic",
            &search_text,
        );
    }

    fn index_agenda(index: &mut Index, agenda: &Agenda) {
        let dates = vec![agenda.date.format("%Y-%m-%d").to_string()];
        let people = vec![agenda.person_slug.clone()];
        let search_text = format!("{} {}", agenda.title, agenda.body);

        index.add(
            EntityRef {
                kind: EntityKind::Agenda,
                id: agenda.id.clone(),
            },
            &agenda.id,
            &dates,
            &people,
            &[],
            None,
            "agenda",
            &search_text,
        );
    }
}

impl Store for FlatFileStore {
    // ── Tasks ────────────────────────────────────────────────────

    fn get_task(&self, id: &str) -> Result<Task> {
        io::read_json(&self.task_path(id))
    }

    fn list_tasks(&self) -> Result<Vec<Task>> {
        let dir = self.data_dir.join("tasks");
        let mut tasks = Vec::new();
        for path in Self::list_dir_files(&dir, "json") {
            match io::read_json::<Task>(&path) {
                Ok(task) => tasks.push(task),
                Err(e) => {
                    let _ = e; // silently skip unparseable files during index rebuild
                }
            }
        }
        tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(tasks)
    }

    fn save_task(&self, task: &Task) -> Result<()> {
        io::write_json(&self.task_path(&task.id), task)
    }

    fn delete_task(&self, id: &str) -> Result<()> {
        let path = self.task_path(id);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("deleting task {}", id))?;
        }
        if let Ok(mut index) = self.index.write() { index.remove(id); }
        Ok(())
    }

    // ── Todos ────────────────────────────────────────────────────

    fn get_todo(&self, id: &str) -> Result<Todo> {
        io::read_json(&self.todo_path(id))
    }

    fn list_todos(&self) -> Result<Vec<Todo>> {
        let dir = self.data_dir.join("todos");
        let mut todos = Vec::new();
        for path in Self::list_dir_files(&dir, "json") {
            match io::read_json::<Todo>(&path) {
                Ok(todo) => todos.push(todo),
                Err(e) => {
                    let _ = e; // silently skip unparseable files during index rebuild
                }
            }
        }
        todos.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(todos)
    }

    fn save_todo(&self, todo: &Todo) -> Result<()> {
        io::write_json(&self.todo_path(&todo.id), todo)
    }

    fn delete_todo(&self, id: &str) -> Result<()> {
        let path = self.todo_path(id);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("deleting todo {}", id))?;
        }
        if let Ok(mut index) = self.index.write() { index.remove(id); }
        Ok(())
    }

    // ── Reminders ────────────────────────────────────────────────

    fn get_reminder(&self, id: &str) -> Result<Reminder> {
        io::read_json(&self.reminder_path(id))
    }

    fn list_reminders(&self) -> Result<Vec<Reminder>> {
        let dir = self.data_dir.join("reminders");
        let mut reminders = Vec::new();
        for path in Self::list_dir_files(&dir, "json") {
            match io::read_json::<Reminder>(&path) {
                Ok(r) => reminders.push(r),
                Err(e) => {
                    let _ = e; // silently skip unparseable files during index rebuild
                }
            }
        }
        reminders.sort_by(|a, b| a.remind_at.cmp(&b.remind_at));
        Ok(reminders)
    }

    fn save_reminder(&self, reminder: &Reminder) -> Result<()> {
        io::write_json(&self.reminder_path(&reminder.id), reminder)
    }

    fn delete_reminder(&self, id: &str) -> Result<()> {
        let path = self.reminder_path(id);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("deleting reminder {}", id))?;
        }
        if let Ok(mut index) = self.index.write() { index.remove(id); }
        Ok(())
    }

    // ── Notes ────────────────────────────────────────────────────

    fn get_note(&self, id: &str) -> Result<Note> {
        io::parse_note(&self.note_path(id))
    }

    fn list_notes(&self) -> Result<Vec<Note>> {
        let dir = self.data_dir.join("notes");
        let mut notes = Vec::new();
        for path in Self::list_dir_files(&dir, "md") {
            match io::parse_note(&path) {
                Ok(note) => notes.push(note),
                Err(e) => {
                    let _ = e; // silently skip unparseable files during index rebuild
                }
            }
        }
        notes.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(notes)
    }

    fn save_note(&self, note: &Note) -> Result<()> {
        io::write_note(&self.note_path(&note.id), note)
    }

    fn delete_note(&self, id: &str) -> Result<()> {
        let path = self.note_path(id);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("deleting note {}", id))?;
        }
        if let Ok(mut index) = self.index.write() { index.remove(id); }
        Ok(())
    }

    // ── People ───────────────────────────────────────────────────

    fn get_person(&self, slug: &str) -> Result<Person> {
        io::read_json(&self.person_path(slug))
    }

    fn list_persons(&self) -> Result<Vec<Person>> {
        let dir = self.data_dir.join("people");
        let mut people = Vec::new();
        for path in Self::list_dir_files(&dir, "json") {
            match io::read_json::<Person>(&path) {
                Ok(p) => people.push(p),
                Err(e) => {
                    let _ = e; // silently skip unparseable files during index rebuild
                }
            }
        }
        people.sort_by(|a, b| a.slug.cmp(&b.slug));
        Ok(people)
    }

    fn save_person(&self, person: &Person) -> Result<()> {
        io::write_json(&self.person_path(&person.slug), person)
    }

    fn delete_person(&self, slug: &str) -> Result<()> {
        let path = self.person_path(slug);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("deleting person {}", slug))?;
        }
        if let Ok(mut index) = self.index.write() { index.remove(slug); }
        Ok(())
    }

    // ── Topics ───────────────────────────────────────────────────

    fn get_topic(&self, slug: &str) -> Result<Topic> {
        io::read_json(&self.topic_path(slug))
    }

    fn list_topics(&self) -> Result<Vec<Topic>> {
        let dir = self.data_dir.join("topics");
        let mut topics = Vec::new();
        for path in Self::list_dir_files(&dir, "json") {
            match io::read_json::<Topic>(&path) {
                Ok(t) => topics.push(t),
                Err(e) => {
                    let _ = e; // silently skip unparseable files during index rebuild
                }
            }
        }
        topics.sort_by(|a, b| a.slug.cmp(&b.slug));
        Ok(topics)
    }

    fn save_topic(&self, topic: &Topic) -> Result<()> {
        io::write_json(&self.topic_path(&topic.slug), topic)
    }

    fn delete_topic(&self, slug: &str) -> Result<()> {
        let path = self.topic_path(slug);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("deleting topic {}", slug))?;
        }
        if let Ok(mut index) = self.index.write() { index.remove(slug); }
        Ok(())
    }

    // ── Agendas ─────────────────────────────────────────────────

    fn get_agenda(&self, id: &str) -> Result<Agenda> {
        let md_path = self.agenda_path(id);
        if md_path.exists() {
            return io::parse_agenda(&md_path);
        }
        anyhow::bail!("agenda {} not found", id)
    }

    fn list_agendas(&self) -> Result<Vec<Agenda>> {
        let dir = self.data_dir.join("agendas");
        let mut agendas = Vec::new();
        for path in Self::list_dir_files(&dir, "md") {
            match io::parse_agenda(&path) {
                Ok(agenda) => agendas.push(agenda),
                Err(_) => {}
            }
        }
        agendas.sort_by(|a, b| b.date.cmp(&a.date));
        Ok(agendas)
    }

    fn list_agendas_for_person(&self, person_slug: &str) -> Result<Vec<Agenda>> {
        let agendas = self.list_agendas()?;
        Ok(agendas
            .into_iter()
            .filter(|a| a.person_slug == person_slug)
            .collect())
    }

    fn save_agenda(&self, agenda: &Agenda) -> Result<()> {
        io::write_agenda(&self.agenda_path(&agenda.id), agenda)
    }

    fn delete_agenda(&self, id: &str) -> Result<()> {
        let md_path = self.agenda_path(id);
        if md_path.exists() {
            fs::remove_file(&md_path)
                .with_context(|| format!("deleting agenda {}", id))?;
        }
        if let Ok(mut index) = self.index.write() { index.remove(id); }
        Ok(())
    }

    // ── Query ────────────────────────────────────────────────────

    fn rebuild_index(&self) -> Result<()> {
        let mut index = self
            .index
            .write()
            .map_err(|e| anyhow::anyhow!("index lock poisoned: {}", e))?;
        index.clear();

        // Index tasks
        let tasks_dir = self.data_dir.join("tasks");
        for path in Self::list_dir_files(&tasks_dir, "json") {
            if let Ok(task) = io::read_json::<Task>(&path) {
                Self::index_task(&mut index, &task);
            }
        }

        // Index todos
        let todos_dir = self.data_dir.join("todos");
        for path in Self::list_dir_files(&todos_dir, "json") {
            if let Ok(todo) = io::read_json::<Todo>(&path) {
                Self::index_todo(&mut index, &todo);
            }
        }

        // Index notes
        let notes_dir = self.data_dir.join("notes");
        for path in Self::list_dir_files(&notes_dir, "md") {
            if let Ok(note) = io::parse_note(&path) {
                Self::index_note(&mut index, &note);
            }
        }

        // Index reminders
        let reminders_dir = self.data_dir.join("reminders");
        for path in Self::list_dir_files(&reminders_dir, "json") {
            if let Ok(reminder) = io::read_json::<Reminder>(&path) {
                Self::index_reminder(&mut index, &reminder);
            }
        }

        // Index agendas
        let agendas_dir = self.data_dir.join("agendas");
        for path in Self::list_dir_files(&agendas_dir, "md") {
            if let Ok(agenda) = io::parse_agenda(&path) {
                Self::index_agenda(&mut index, &agenda);
            }
        }

        // Collect all referenced people and topics from the index
        // so we can auto-create records for any that don't exist on disk.
        let referenced_people: std::collections::HashSet<String> =
            index.by_person.keys().cloned().collect();
        let referenced_topics: std::collections::HashSet<String> =
            index.by_topic.keys().cloned().collect();

        // Auto-create missing Person records
        let now = chrono::Utc::now();
        for slug in &referenced_people {
            let person_path = self.person_path(slug);
            if !person_path.exists() {
                let person = Person {
                    slug: slug.clone(),
                    created_at: now,
                    pinned: false,
                    archived: false,
                    metadata: Default::default(),
                    tags: Vec::new(),
                };
                let _ = io::write_json(&person_path, &person);
            }
        }

        // Auto-create missing Topic records
        for slug in &referenced_topics {
            let topic_path = self.topic_path(slug);
            if !topic_path.exists() {
                let topic = Topic {
                    slug: slug.clone(),
                    display_name: String::new(),
                    aliases: Vec::new(),
                    created_at: now,
                    description: String::new(),
                    metadata: Default::default(),
                };
                let _ = io::write_json(&topic_path, &topic);
            }
        }

        // Index people
        let people_dir = self.data_dir.join("people");
        for path in Self::list_dir_files(&people_dir, "json") {
            if let Ok(person) = io::read_json::<Person>(&path) {
                Self::index_person(&mut index, &person);
            }
        }

        // Index topics
        let topics_dir = self.data_dir.join("topics");
        for path in Self::list_dir_files(&topics_dir, "json") {
            if let Ok(topic) = io::read_json::<Topic>(&path) {
                Self::index_topic(&mut index, &topic);
            }
        }

        Ok(())
    }

    fn entities_by_date(&self, date: &str) -> Vec<EntityRef> {
        let index = match self.index.read() {
            Ok(idx) => idx,
            Err(_) => return Vec::new(),
        };

        let Some(ids) = index.by_date.get(date) else {
            return Vec::new();
        };

        ids.iter()
            .filter_map(|id| index.by_id.get(id).cloned())
            .collect()
    }

    fn get_memory(&self, slug: &str) -> Vec<EntityRef> {
        let index = match self.index.read() {
            Ok(idx) => idx,
            Err(_) => return Vec::new(),
        };

        let empty = std::collections::HashSet::new();
        let person_ids = index.by_person.get(slug).unwrap_or(&empty);
        let topic_ids = index.by_topic.get(slug).unwrap_or(&empty);

        person_ids.union(topic_ids)
            .filter_map(|id| index.by_id.get(id).cloned())
            .collect()
    }

    fn search(&self, query: &str) -> Vec<EntityRef> {
        let index = match self.index.read() {
            Ok(idx) => idx,
            Err(_) => return Vec::new(),
        };

        let scored = index.fts.fuzzy_search(query);
        scored
            .iter()
            .filter_map(|(id, _score)| index.by_id.get(id).cloned())
            .collect()
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, Utc};
    use tempfile::TempDir;

    fn test_store() -> (TempDir, FlatFileStore) {
        let dir = TempDir::new().unwrap();
        let store = FlatFileStore::new(dir.path()).unwrap();
        (dir, store)
    }

    #[test]
    fn creates_subdirectories() {
        let (dir, _store) = test_store();
        assert!(dir.path().join("tasks").is_dir());
        assert!(dir.path().join("todos").is_dir());
        assert!(dir.path().join("notes").is_dir());
        assert!(dir.path().join("reminders").is_dir());
        assert!(dir.path().join("people").is_dir());
        assert!(dir.path().join("topics").is_dir());
        assert!(dir.path().join("agendas").is_dir());
    }

    #[test]
    fn task_crud() {
        let (_dir, store) = test_store();
        let now = Utc::now();
        let task = Task {
            id: "task1".to_string(),
            title: "Test task".to_string(),
            description: String::new(),
            status: TaskStatus::Todo,
            created_at: now,
            updated_at: now,
            due_date: Some(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()),
            due_time: None,
            priority: Priority::None,
            private: false,
            pinned: false,
            archived: false,
            created_dir: String::new(),
            refs: Refs::default(),
            status_history: Vec::new(),
        };

        store.save_task(&task).unwrap();
        let loaded = store.get_task("task1").unwrap();
        assert_eq!(loaded.title, "Test task");
        assert_eq!(loaded.status, TaskStatus::Todo);

        let list = store.list_tasks().unwrap();
        assert_eq!(list.len(), 1);

        store.delete_task("task1").unwrap();
        assert!(store.get_task("task1").is_err());
    }

    #[test]
    fn todo_crud() {
        let (_dir, store) = test_store();
        let now = Utc::now();
        let todo = Todo {
            id: "todo1".to_string(),
            title: "Buy milk".to_string(),
            done: false,
            created_at: now,
            done_at: None,
            due_date: None,
            due_time: None,
            refs: Refs::default(),
        };

        store.save_todo(&todo).unwrap();
        let loaded = store.get_todo("todo1").unwrap();
        assert_eq!(loaded.title, "Buy milk");

        store.delete_todo("todo1").unwrap();
        assert!(store.get_todo("todo1").is_err());
    }

    #[test]
    fn note_crud() {
        let (_dir, store) = test_store();
        let now = Utc::now();
        let note = Note {
            id: "note1".to_string(),
            title: "My note".to_string(),
            created_at: now,
            updated_at: now,
            private: false,
            pinned: false,
            archived: false,
            created_dir: String::new(),
            refs: Refs::default(),
            body: "This is the body.\n".to_string(),
        };

        store.save_note(&note).unwrap();
        let loaded = store.get_note("note1").unwrap();
        assert_eq!(loaded.title, "My note");
        assert_eq!(loaded.body, "This is the body.\n");

        let list = store.list_notes().unwrap();
        assert_eq!(list.len(), 1);

        store.delete_note("note1").unwrap();
        assert!(store.get_note("note1").is_err());
    }

    #[test]
    fn reminder_crud() {
        let (_dir, store) = test_store();
        let now = Utc::now();
        let reminder = Reminder {
            id: "rem1".to_string(),
            title: "Call dentist".to_string(),
            remind_at: now,
            dismissed: false,
            created_at: now,
            refs: Refs::default(),
        };

        store.save_reminder(&reminder).unwrap();
        let loaded = store.get_reminder("rem1").unwrap();
        assert_eq!(loaded.title, "Call dentist");

        store.delete_reminder("rem1").unwrap();
        assert!(store.get_reminder("rem1").is_err());
    }

    #[test]
    fn person_crud() {
        let (_dir, store) = test_store();
        let now = Utc::now();
        let mut meta = std::collections::HashMap::new();
        meta.insert("name".to_string(), "Alice Smith".to_string());
        let person = Person {
            slug: "alice".to_string(),
            created_at: now,
            pinned: false,
            archived: false,
            metadata: meta,
            tags: Vec::new(),
        };

        store.save_person(&person).unwrap();
        let loaded = store.get_person("alice").unwrap();
        assert_eq!(loaded.metadata.get("name").unwrap(), "Alice Smith");

        let list = store.list_persons().unwrap();
        assert_eq!(list.len(), 1);

        store.delete_person("alice").unwrap();
        assert!(store.get_person("alice").is_err());
    }

    #[test]
    fn topic_crud() {
        let (_dir, store) = test_store();
        let now = Utc::now();
        let topic = Topic {
            slug: "rust".to_string(),
            display_name: "Rust".to_string(),
            aliases: Vec::new(),
            created_at: now,
            description: "The Rust programming language".to_string(),
            metadata: std::collections::HashMap::new(),
        };

        store.save_topic(&topic).unwrap();
        let loaded = store.get_topic("rust").unwrap();
        assert_eq!(loaded.description, "The Rust programming language");

        store.delete_topic("rust").unwrap();
        assert!(store.get_topic("rust").is_err());
    }

    #[test]
    fn rebuild_index_and_search() {
        let (_dir, store) = test_store();
        let now = Utc::now();

        let task = Task {
            id: "t1".to_string(),
            title: "Buy groceries @bob #shopping".to_string(),
            description: String::new(),
            status: TaskStatus::Todo,
            created_at: now,
            updated_at: now,
            due_date: Some(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()),
            due_time: None,
            priority: Priority::None,
            private: false,
            pinned: false,
            archived: false,
            created_dir: String::new(),
            refs: Refs::default(),
            status_history: Vec::new(),
        };
        store.save_task(&task).unwrap();

        let note = Note {
            id: "n1".to_string(),
            title: "Meeting notes".to_string(),
            created_at: now,
            updated_at: now,
            private: false,
            pinned: false,
            archived: false,
            created_dir: String::new(),
            refs: Refs::default(),
            body: "Discussed #shopping list with @bob.\n".to_string(),
        };
        store.save_note(&note).unwrap();

        store.rebuild_index().unwrap();

        // FTS search
        let results = store.search("groceries");
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].kind, EntityKind::Task));

        let results = store.search("meeting");
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].kind, EntityKind::Note));

        // Memory lookup (by person)
        let refs = store.get_memory("bob");
        assert_eq!(refs.len(), 2); // task + note both mention @bob

        // Memory lookup (by topic)
        let refs = store.get_memory("shopping");
        assert_eq!(refs.len(), 2); // task + note both reference #shopping

        // Entities by date
        let refs = store.entities_by_date("2026-03-01");
        assert!(!refs.is_empty());
    }

    #[test]
    fn rebuild_index_includes_sink_note() {
        // A daily sink note stored as notes/sink-2026-02-27.md should
        // have its body indexed for full-text search.
        let (_dir, store) = test_store();
        let now = Utc::now();

        let note = Note {
            id: "sink-2026-02-27".to_string(),
            title: "Sink 2026-02-27".to_string(),
            created_at: now,
            updated_at: now,
            private: false,
            pinned: false,
            archived: false,
            created_dir: String::new(),
            refs: Refs::default(),
            body: "remember @carol about #project\n".to_string(),
        };
        store.save_note(&note).unwrap();

        store.rebuild_index().unwrap();

        let results = store.search("remember");
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].kind, EntityKind::Note));

        let refs = store.get_memory("carol");
        assert_eq!(refs.len(), 1);

        let refs = store.get_memory("project");
        assert_eq!(refs.len(), 1);
    }

    #[test]
    fn agenda_crud() {
        let (_dir, store) = test_store();
        let now = Utc::now();
        let agenda = Agenda {
            id: "agenda1".to_string(),
            title: "1:1 with Alice".to_string(),
            person_slug: "alice".to_string(),
            date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            created_at: now,
            updated_at: now,
            body: String::new(),
            refs: Default::default(),
        };

        store.save_agenda(&agenda).unwrap();
        let loaded = store.get_agenda("agenda1").unwrap();
        assert_eq!(loaded.title, "1:1 with Alice");
        assert_eq!(loaded.person_slug, "alice");

        let list = store.list_agendas().unwrap();
        assert_eq!(list.len(), 1);

        let for_person = store.list_agendas_for_person("alice").unwrap();
        assert_eq!(for_person.len(), 1);

        let for_nobody = store.list_agendas_for_person("bob").unwrap();
        assert!(for_nobody.is_empty());

        store.delete_agenda("agenda1").unwrap();
        assert!(store.get_agenda("agenda1").is_err());
    }

    #[test]
    fn delete_nonexistent_is_ok() {
        let (_dir, store) = test_store();
        // Deleting something that doesn't exist should not error
        assert!(store.delete_task("nonexistent").is_ok());
        assert!(store.delete_todo("nonexistent").is_ok());
        assert!(store.delete_note("nonexistent").is_ok());
        assert!(store.delete_reminder("nonexistent").is_ok());
        assert!(store.delete_person("nonexistent").is_ok());
        assert!(store.delete_topic("nonexistent").is_ok());
        assert!(store.delete_agenda("nonexistent").is_ok());
    }

    #[test]
    fn list_empty_directory() {
        let (_dir, store) = test_store();
        assert!(store.list_tasks().unwrap().is_empty());
        assert!(store.list_todos().unwrap().is_empty());
        assert!(store.list_notes().unwrap().is_empty());
        assert!(store.list_reminders().unwrap().is_empty());
        assert!(store.list_persons().unwrap().is_empty());
        assert!(store.list_topics().unwrap().is_empty());
        assert!(store.list_agendas().unwrap().is_empty());
    }

}
