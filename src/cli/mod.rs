pub mod output;

use std::io::Write;
use std::sync::Arc;

use anyhow::anyhow;
use chrono::{NaiveDate, Utc};
use clap::{Parser, Subcommand};

use crate::domain::{new_id, Agenda, EntityKind, EntityRef, Note, Person, Refs, Tag, Task, TaskStatus};
use crate::store::Store;

use output::{
    AgendaOutput, DeleteResult, NoteOutput, PersonOutput, SearchResult, TagOutput, TaskOutput,
    print_json,
};

// ── Clap definitions ─────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "crumbs", about = "Personal productivity system")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
    /// Use a named vault (e.g. --vault=work uses work.db in the data dir)
    #[arg(long, global = true)]
    pub vault: Option<String>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Launch the TUI
    Tui {
        #[arg(long)]
        tag: Option<String>,
    },
    /// Manage tasks
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
    /// Manage notes
    Note {
        #[command(subcommand)]
        action: NoteAction,
    },
    /// Manage people
    Person {
        #[command(subcommand)]
        action: PersonAction,
    },
    /// Manage tags
    Tag {
        #[command(subcommand)]
        action: TagAction,
    },
    /// Manage agendas
    Agenda {
        #[command(subcommand)]
        action: AgendaAction,
    },
    /// Full-text search
    Search {
        query: String,
        #[arg(long)]
        tag: Option<String>,
    },
    /// Rapid-capture sink entry
    Sink {
        text: String,
    },
    /// Show entities for today
    Today {
        #[arg(long)]
        tag: Option<String>,
    },
}

// ── Task subcommands ──────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub(crate) enum TaskAction {
    /// List tasks
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        archived: bool,
    },
    /// Get a single task by ID
    Get { id: String },
    /// Create a new task
    Add {
        title: String,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        priority: Option<String>,
        #[arg(long)]
        due: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long, name = "body-stdin")]
        body_stdin: bool,
        #[arg(long)]
        private: bool,
        #[arg(long)]
        pinned: bool,
    },
    /// Update a task
    Update {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        priority: Option<String>,
        #[arg(long)]
        due: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long, name = "body-stdin")]
        body_stdin: bool,
        #[arg(long)]
        private: Option<bool>,
        #[arg(long)]
        pinned: Option<bool>,
        #[arg(long)]
        archived: Option<bool>,
    },
    /// Delete a task
    Delete { id: String },
}

// ── Note subcommands ──────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub(crate) enum NoteAction {
    /// List notes
    List {
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        archived: bool,
    },
    /// Get a single note by ID
    Get { id: String },
    /// Create a new note
    Add {
        title: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long, name = "body-stdin")]
        body_stdin: bool,
        #[arg(long)]
        private: bool,
        #[arg(long)]
        pinned: bool,
    },
    /// Update a note
    Update {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long, name = "body-stdin")]
        body_stdin: bool,
        #[arg(long)]
        private: Option<bool>,
        #[arg(long)]
        pinned: Option<bool>,
        #[arg(long)]
        archived: Option<bool>,
    },
    /// Delete a note
    Delete { id: String },
}

// ── Person subcommands ────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub(crate) enum PersonAction {
    /// List people
    List {
        #[arg(long)]
        archived: bool,
    },
    /// Get a single person by slug
    Get { slug: String },
    /// Add a new person
    Add {
        slug: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, value_parser = parse_key_val, number_of_values = 1)]
        meta: Vec<(String, String)>,
    },
    /// Update a person
    Update {
        slug: String,
        #[arg(long, value_parser = parse_key_val, number_of_values = 1)]
        meta: Vec<(String, String)>,
        #[arg(long)]
        pinned: Option<bool>,
        #[arg(long)]
        archived: Option<bool>,
    },
    /// Rename a person's slug
    Rename {
        old_slug: String,
        new_slug: String,
    },
    /// Delete a person
    Delete { slug: String },
}

// ── Tag subcommands ───────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub(crate) enum TagAction {
    /// List all tags
    List,
    /// Add a new tag
    Add { slug: String },
    /// Delete a tag
    Delete { slug: String },
}

// ── Agenda subcommands ────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub(crate) enum AgendaAction {
    /// List agendas
    List {
        #[arg(long)]
        person: Option<String>,
    },
    /// Get a single agenda by ID
    Get { id: String },
    /// Create a new agenda
    Add {
        title: String,
        #[arg(long)]
        person: String,
        #[arg(long)]
        date: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long, name = "body-stdin")]
        body_stdin: bool,
    },
    /// Update an agenda
    Update {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        date: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long, name = "body-stdin")]
        body_stdin: bool,
    },
    /// Delete an agenda
    Delete { id: String },
}

// ── Helper functions ──────────────────────────────────────────────────────────

fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid key=value: no `=` found in `{s}`"))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

fn read_stdin_body() -> anyhow::Result<String> {
    use std::io::Read;
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}

fn resolve_body(body: Option<String>, body_stdin: bool) -> anyhow::Result<Option<String>> {
    if body_stdin {
        Ok(Some(read_stdin_body()?))
    } else {
        Ok(body)
    }
}

fn parse_status(s: &str) -> anyhow::Result<TaskStatus> {
    TaskStatus::from_str_loose(s)
        .ok_or_else(|| anyhow!("unknown status: {s}"))
}

fn parse_priority(s: &str) -> anyhow::Result<crate::domain::Priority> {
    s.parse::<crate::domain::Priority>()
        .map_err(|e| anyhow!("{e}"))
}

fn parse_date(s: &str) -> anyhow::Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| anyhow!("invalid date: {s} (expected YYYY-MM-DD)"))
}

fn resolve_title(store: &dyn Store, entity_ref: &EntityRef) -> Option<String> {
    match entity_ref.kind {
        EntityKind::Task => store.get_task(&entity_ref.id).ok().map(|t| t.title),
        EntityKind::Note => store.get_note(&entity_ref.id).ok().map(|n| n.title),
        EntityKind::Person => store
            .get_person(&entity_ref.id)
            .ok()
            .map(|p| p.display_name()),
        EntityKind::Tag => Some(format!("#{}", entity_ref.id)),
        EntityKind::Agenda => store.get_agenda(&entity_ref.id).ok().map(|a| a.title),
    }
}

fn entity_kind_str(kind: &EntityKind) -> &'static str {
    match kind {
        EntityKind::Task => "task",
        EntityKind::Note => "note",
        EntityKind::Person => "person",
        EntityKind::Tag => "tag",
        EntityKind::Agenda => "agenda",
    }
}

// ── execute() ────────────────────────────────────────────────────────────────

/// Execute a CLI command, writing JSON output to stdout.
pub fn execute(command: Command, store: Arc<dyn Store>) -> anyhow::Result<()> {
    execute_to(&mut std::io::stdout(), command, store)
}

/// Execute a CLI command, writing JSON output to `out`.
pub fn execute_to(out: &mut dyn Write, command: Command, store: Arc<dyn Store>) -> anyhow::Result<()> {
    match command {
        Command::Tui { .. } => {
            // Should not reach here — handled in main.rs
            unreachable!("Tui command should be handled in main");
        }

        // ── Tasks ─────────────────────────────────────────────────────────────
        Command::Task { action } => match action {
            TaskAction::List { status, tag, archived } => {
                let mut tasks = store.list_tasks()?;
                if !archived {
                    tasks.retain(|t| !t.archived);
                }
                if let Some(ref s) = status {
                    let wanted = parse_status(s)?;
                    tasks.retain(|t| t.status == wanted);
                }
                if let Some(ref tag_slug) = tag {
                    tasks.retain(|t| t.refs.tags.contains(tag_slug));
                }
                let items: Vec<TaskOutput> = tasks.into_iter().map(TaskOutput::from).collect();
                print_json(out, &items)
            }
            TaskAction::Get { id } => {
                let task = store.get_task(&id)?;
                print_json(out,&TaskOutput::from(task))
            }
            TaskAction::Add {
                title,
                status,
                priority,
                due,
                body,
                body_stdin,
                private,
                pinned,
            } => {
                let now = Utc::now();
                let mut task = Task {
                    id: new_id(),
                    title,
                    status: if let Some(s) = status {
                        parse_status(&s)?
                    } else {
                        TaskStatus::Backlog
                    },
                    priority: if let Some(p) = priority {
                        parse_priority(&p)?
                    } else {
                        crate::domain::Priority::None
                    },
                    due_date: if let Some(d) = due {
                        Some(parse_date(&d)?)
                    } else {
                        None
                    },
                    private,
                    pinned,
                    created_at: now,
                    updated_at: now,
                    ..Task::default()
                };
                if let Some(body_text) = resolve_body(body, body_stdin)? {
                    task.description = body_text;
                }
                store.save_task(&task)?;
                print_json(out,&TaskOutput::from(task))
            }
            TaskAction::Update {
                id,
                title,
                status,
                priority,
                due,
                body,
                body_stdin,
                private,
                pinned,
                archived,
            } => {
                let mut task = store.get_task(&id)?;
                if let Some(t) = title {
                    task.title = t;
                }
                if let Some(s) = status {
                    task.status = parse_status(&s)?;
                }
                if let Some(p) = priority {
                    task.priority = parse_priority(&p)?;
                }
                if let Some(d) = due {
                    task.due_date = Some(parse_date(&d)?);
                }
                if let Some(body_text) = resolve_body(body, body_stdin)? {
                    task.description = body_text;
                }
                if let Some(v) = private {
                    task.private = v;
                }
                if let Some(v) = pinned {
                    task.pinned = v;
                }
                if let Some(v) = archived {
                    task.archived = v;
                }
                task.updated_at = Utc::now();
                store.save_task(&task)?;
                print_json(out,&TaskOutput::from(task))
            }
            TaskAction::Delete { id } => {
                store.delete_task(&id)?;
                print_json(out,&DeleteResult { deleted: id })
            }
        },

        // ── Notes ─────────────────────────────────────────────────────────────
        Command::Note { action } => match action {
            NoteAction::List { tag, archived } => {
                let mut notes = store.list_notes()?;
                if !archived {
                    notes.retain(|n| !n.archived);
                }
                if let Some(ref tag_slug) = tag {
                    notes.retain(|n| n.refs.tags.contains(tag_slug));
                }
                let items: Vec<NoteOutput> = notes.into_iter().map(NoteOutput::from).collect();
                print_json(out, &items)
            }
            NoteAction::Get { id } => {
                let note = store.get_note(&id)?;
                print_json(out,&NoteOutput::from(note))
            }
            NoteAction::Add {
                title,
                body,
                body_stdin,
                private,
                pinned,
            } => {
                let now = Utc::now();
                let mut note = Note {
                    id: new_id(),
                    title,
                    private,
                    pinned,
                    created_at: now,
                    updated_at: now,
                    archived: false,
                    created_dir: String::new(),
                    refs: Refs::default(),
                    body: String::new(),
                };
                if let Some(body_text) = resolve_body(body, body_stdin)? {
                    note.body = body_text;
                }
                store.save_note(&note)?;
                print_json(out,&NoteOutput::from(note))
            }
            NoteAction::Update {
                id,
                title,
                body,
                body_stdin,
                private,
                pinned,
                archived,
            } => {
                let mut note = store.get_note(&id)?;
                if let Some(t) = title {
                    note.title = t;
                }
                if let Some(body_text) = resolve_body(body, body_stdin)? {
                    note.body = body_text;
                }
                if let Some(v) = private {
                    note.private = v;
                }
                if let Some(v) = pinned {
                    note.pinned = v;
                }
                if let Some(v) = archived {
                    note.archived = v;
                }
                note.updated_at = Utc::now();
                store.save_note(&note)?;
                print_json(out,&NoteOutput::from(note))
            }
            NoteAction::Delete { id } => {
                store.delete_note(&id)?;
                print_json(out,&DeleteResult { deleted: id })
            }
        },

        // ── People ────────────────────────────────────────────────────────────
        Command::Person { action } => match action {
            PersonAction::List { archived } => {
                let mut people = store.list_persons()?;
                if !archived {
                    people.retain(|p| !p.archived);
                }
                let items: Vec<PersonOutput> = people.into_iter().map(PersonOutput::from).collect();
                print_json(out, &items)
            }
            PersonAction::Get { slug } => {
                let person = store.get_person(&slug)?;
                print_json(out,&PersonOutput::from(person))
            }
            PersonAction::Add { slug, name, meta } => {
                let now = Utc::now();
                let mut metadata = std::collections::HashMap::new();
                if let Some(n) = name {
                    metadata.insert("name".to_string(), n);
                }
                for (k, v) in meta {
                    metadata.insert(k, v);
                }
                let person = Person {
                    slug,
                    created_at: now,
                    pinned: false,
                    archived: false,
                    metadata,
                };
                store.save_person(&person)?;
                print_json(out,&PersonOutput::from(person))
            }
            PersonAction::Update {
                slug,
                meta,
                pinned,
                archived,
            } => {
                let mut person = store.get_person(&slug)?;
                for (k, v) in meta {
                    person.metadata.insert(k, v);
                }
                if let Some(v) = pinned {
                    person.pinned = v;
                }
                if let Some(v) = archived {
                    person.archived = v;
                }
                store.save_person(&person)?;
                print_json(out,&PersonOutput::from(person))
            }
            PersonAction::Rename { old_slug, new_slug } => {
                store.rename_person(&old_slug, &new_slug)?;
                let person = store.get_person(&new_slug)?;
                print_json(out,&PersonOutput::from(person))
            }
            PersonAction::Delete { slug } => {
                store.delete_person(&slug)?;
                print_json(out,&DeleteResult { deleted: slug })
            }
        },

        // ── Tags ──────────────────────────────────────────────────────────────
        Command::Tag { action } => match action {
            TagAction::List => {
                let tags = store.list_tags()?;
                let items: Vec<TagOutput> = tags.into_iter().map(TagOutput::from).collect();
                print_json(out, &items)
            }
            TagAction::Add { slug } => {
                let tag = Tag {
                    slug,
                    created_at: Utc::now(),
                };
                store.save_tag(&tag)?;
                print_json(out,&TagOutput::from(tag))
            }
            TagAction::Delete { slug } => {
                store.delete_tag(&slug)?;
                print_json(out,&DeleteResult { deleted: slug })
            }
        },

        // ── Agendas ───────────────────────────────────────────────────────────
        Command::Agenda { action } => match action {
            AgendaAction::List { person } => {
                let agendas = if let Some(ref slug) = person {
                    store.list_agendas_for_person(slug)?
                } else {
                    store.list_agendas()?
                };
                let items: Vec<AgendaOutput> = agendas.into_iter().map(AgendaOutput::from).collect();
                print_json(out, &items)
            }
            AgendaAction::Get { id } => {
                let agenda = store.get_agenda(&id)?;
                print_json(out,&AgendaOutput::from(agenda))
            }
            AgendaAction::Add {
                title,
                person,
                date,
                body,
                body_stdin,
            } => {
                let now = Utc::now();
                let mut agenda = Agenda {
                    id: new_id(),
                    title,
                    person_slug: person,
                    date: parse_date(&date)?,
                    created_at: now,
                    updated_at: now,
                    body: String::new(),
                    refs: Refs::default(),
                };
                if let Some(body_text) = resolve_body(body, body_stdin)? {
                    agenda.body = body_text;
                }
                store.save_agenda(&agenda)?;
                print_json(out,&AgendaOutput::from(agenda))
            }
            AgendaAction::Update {
                id,
                title,
                date,
                body,
                body_stdin,
            } => {
                let mut agenda = store.get_agenda(&id)?;
                if let Some(t) = title {
                    agenda.title = t;
                }
                if let Some(d) = date {
                    agenda.date = parse_date(&d)?;
                }
                if let Some(body_text) = resolve_body(body, body_stdin)? {
                    agenda.body = body_text;
                }
                agenda.updated_at = Utc::now();
                store.save_agenda(&agenda)?;
                print_json(out,&AgendaOutput::from(agenda))
            }
            AgendaAction::Delete { id } => {
                store.delete_agenda(&id)?;
                print_json(out,&DeleteResult { deleted: id })
            }
        },

        // ── Search ────────────────────────────────────────────────────────────
        Command::Search { query, tag } => {
            let refs = store.search(&query);
            let results: Vec<SearchResult> = refs
                .iter()
                .filter_map(|r| {
                    let title = resolve_title(store.as_ref(), r)?;
                    // Apply tag filter: only tasks/notes/agendas can have tags
                    if let Some(ref tag_slug) = tag {
                        let has_tag = match r.kind {
                            EntityKind::Task => store
                                .get_task(&r.id)
                                .ok()
                                .map(|t| t.refs.tags.contains(tag_slug))
                                .unwrap_or(false),
                            EntityKind::Note => store
                                .get_note(&r.id)
                                .ok()
                                .map(|n| n.refs.tags.contains(tag_slug))
                                .unwrap_or(false),
                            EntityKind::Agenda => store
                                .get_agenda(&r.id)
                                .ok()
                                .map(|a| a.refs.tags.contains(tag_slug))
                                .unwrap_or(false),
                            _ => false,
                        };
                        if !has_tag {
                            return None;
                        }
                    }
                    Some(SearchResult {
                        kind: entity_kind_str(&r.kind).to_string(),
                        id: r.id.clone(),
                        title,
                    })
                })
                .collect();
            print_json(out,&results)
        }

        // ── Sink ──────────────────────────────────────────────────────────────
        Command::Sink { .. } => {
            Err(anyhow!("sink command not yet implemented"))
        }

        // ── Today ─────────────────────────────────────────────────────────────
        Command::Today { tag } => {
            let today = chrono::Local::now().date_naive().format("%Y-%m-%d").to_string();
            let refs = store.entities_by_date(&today);
            let results: Vec<SearchResult> = refs
                .iter()
                .filter_map(|r| {
                    let title = resolve_title(store.as_ref(), r)?;
                    if let Some(ref tag_slug) = tag {
                        let has_tag = match r.kind {
                            EntityKind::Task => store
                                .get_task(&r.id)
                                .ok()
                                .map(|t| t.refs.tags.contains(tag_slug))
                                .unwrap_or(false),
                            EntityKind::Note => store
                                .get_note(&r.id)
                                .ok()
                                .map(|n| n.refs.tags.contains(tag_slug))
                                .unwrap_or(false),
                            EntityKind::Agenda => store
                                .get_agenda(&r.id)
                                .ok()
                                .map(|a| a.refs.tags.contains(tag_slug))
                                .unwrap_or(false),
                            _ => false,
                        };
                        if !has_tag {
                            return None;
                        }
                    }
                    Some(SearchResult {
                        kind: entity_kind_str(&r.kind).to_string(),
                        id: r.id.clone(),
                        title,
                    })
                })
                .collect();
            print_json(out,&results)
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use crate::store::SqliteStore;

    use super::*;

    /// Create a fresh in-memory (tempdir) store for each test.
    fn test_store() -> (Arc<dyn Store>, TempDir) {
        let dir = TempDir::new().expect("failed to create temp dir for test store");
        let db = dir.path().join("test.db");
        let store: Arc<dyn Store> = Arc::new(
            SqliteStore::new(&db).expect("failed to open SQLite test store"),
        );
        (store, dir)
    }

    /// Run a command against a store and return the captured JSON output as a string.
    fn run(store: Arc<dyn Store>, cmd: Command) -> String {
        let mut buf = Vec::<u8>::new();
        execute_to(&mut buf, cmd, store).expect("CLI command execution failed");
        String::from_utf8(buf).expect("CLI output was not valid UTF-8")
    }

    /// Parse JSON output into a `serde_json::Value`.
    fn json(s: &str) -> serde_json::Value {
        serde_json::from_str(s.trim()).expect("failed to parse CLI output as JSON")
    }

    // ── Tasks ─────────────────────────────────────────────────────────────────

    #[test]
    fn task_add_and_get_roundtrip() {
        let (store, _dir) = test_store();
        let out = run(Arc::clone(&store), Command::Task {
            action: TaskAction::Add {
                title: "Buy milk".into(),
                status: None,
                priority: Some("high".into()),
                due: Some("2026-05-01".into()),
                body: Some("from the corner shop".into()),
                body_stdin: false,
                private: false,
                pinned: false,
            },
        });
        let v = json(&out);
        let id = v["id"].as_str().expect("task output missing 'id' field").to_owned();
        assert_eq!(v["title"], "Buy milk");
        assert_eq!(v["priority"], "High");
        assert_eq!(v["due_date"], "2026-05-01");
        assert_eq!(v["description"], "from the corner shop");
        assert_eq!(v["status"], "backlog");

        let got = run(Arc::clone(&store), Command::Task {
            action: TaskAction::Get { id },
        });
        let v2 = json(&got);
        assert_eq!(v2["title"], "Buy milk");
        assert_eq!(v2["priority"], "High");
    }

    #[test]
    fn task_update_only_changes_specified_fields() {
        let (store, _dir) = test_store();
        let created = json(&run(Arc::clone(&store), Command::Task {
            action: TaskAction::Add {
                title: "Original title".into(),
                status: None,
                priority: None,
                due: None,
                body: None,
                body_stdin: false,
                private: false,
                pinned: false,
            },
        }));
        let id = created["id"].as_str().expect("created task missing 'id'").to_owned();

        let updated = json(&run(Arc::clone(&store), Command::Task {
            action: TaskAction::Update {
                id: id.clone(),
                title: None,
                status: Some("done".into()),
                priority: None,
                due: None,
                body: None,
                body_stdin: false,
                private: None,
                pinned: None,
                archived: None,
            },
        }));
        // status changed
        assert_eq!(updated["status"], "done");
        // title unchanged
        assert_eq!(updated["title"], "Original title");
    }

    #[test]
    fn task_list_filters_by_status() {
        let (store, _dir) = test_store();
        for title in &["Alpha", "Beta", "Gamma"] {
            run(Arc::clone(&store), Command::Task {
                action: TaskAction::Add {
                    title: (*title).into(),
                    status: None,
                    priority: None,
                    due: None,
                    body: None,
                    body_stdin: false,
                    private: false,
                    pinned: false,
                },
            });
        }
        // Mark Alpha as done
        let all = json(&run(Arc::clone(&store), Command::Task {
            action: TaskAction::List { status: None, tag: None, archived: false },
        }));
        let alpha_id = all.as_array().expect("task list should be an array")
            .iter()
            .find(|t| t["title"] == "Alpha")
            .expect("task 'Alpha' not found in list")["id"]
            .as_str().expect("task missing 'id'").to_owned();
        run(Arc::clone(&store), Command::Task {
            action: TaskAction::Update {
                id: alpha_id,
                title: None,
                status: Some("done".into()),
                priority: None,
                due: None,
                body: None,
                body_stdin: false,
                private: None,
                pinned: None,
                archived: None,
            },
        });

        let done = json(&run(Arc::clone(&store), Command::Task {
            action: TaskAction::List { status: Some("done".into()), tag: None, archived: false },
        }));
        let done_arr = done.as_array().expect("filtered task list should be an array");
        assert_eq!(done_arr.len(), 1);
        assert_eq!(done_arr[0]["title"], "Alpha");
    }

    #[test]
    fn task_list_filters_by_tag() {
        let (store, _dir) = test_store();
        // Add a task with refs manually via the store to set a tag
        let mut task = crate::domain::Task {
            id: crate::domain::new_id(),
            title: "Tagged task".into(),
            refs: crate::domain::Refs {
                tags: vec!["work".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        task.created_at = chrono::Utc::now();
        task.updated_at = chrono::Utc::now();
        store.save_task(&task).expect("failed to save tagged task to store");
        // Also add an untagged task via CLI
        run(Arc::clone(&store), Command::Task {
            action: TaskAction::Add {
                title: "Untagged task".into(),
                status: None, priority: None, due: None,
                body: None, body_stdin: false, private: false, pinned: false,
            },
        });

        let filtered = json(&run(Arc::clone(&store), Command::Task {
            action: TaskAction::List { status: None, tag: Some("work".into()), archived: false },
        }));
        let arr = filtered.as_array().expect("tag-filtered task list should be an array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["title"], "Tagged task");
    }

    #[test]
    fn task_delete_returns_deleted_id() {
        let (store, _dir) = test_store();
        let created = json(&run(Arc::clone(&store), Command::Task {
            action: TaskAction::Add {
                title: "Temporary".into(),
                status: None, priority: None, due: None,
                body: None, body_stdin: false, private: false, pinned: false,
            },
        }));
        let id = created["id"].as_str().expect("created task missing 'id'").to_owned();

        let del = json(&run(Arc::clone(&store), Command::Task {
            action: TaskAction::Delete { id: id.clone() },
        }));
        assert_eq!(del["deleted"], id);

        // Gone from the store
        assert!(store.get_task(&id).is_err());
    }

    // ── Notes ─────────────────────────────────────────────────────────────────

    #[test]
    fn note_add_includes_body_in_output() {
        let (store, _dir) = test_store();
        let out = json(&run(Arc::clone(&store), Command::Note {
            action: NoteAction::Add {
                title: "My note".into(),
                body: Some("line one\nline two".into()),
                body_stdin: false,
                private: false,
                pinned: false,
            },
        }));
        assert_eq!(out["title"], "My note");
        assert_eq!(out["body"], "line one\nline two");
    }

    #[test]
    fn note_update_preserves_body_when_not_specified() {
        let (store, _dir) = test_store();
        let created = json(&run(Arc::clone(&store), Command::Note {
            action: NoteAction::Add {
                title: "Note".into(),
                body: Some("original body".into()),
                body_stdin: false,
                private: false,
                pinned: false,
            },
        }));
        let id = created["id"].as_str().expect("created note missing 'id'").to_owned();

        let updated = json(&run(Arc::clone(&store), Command::Note {
            action: NoteAction::Update {
                id,
                title: Some("Renamed".into()),
                body: None,
                body_stdin: false,
                private: None,
                pinned: None,
                archived: None,
            },
        }));
        assert_eq!(updated["title"], "Renamed");
        assert_eq!(updated["body"], "original body");
    }

    // ── People ────────────────────────────────────────────────────────────────

    #[test]
    fn person_add_and_rename() {
        let (store, _dir) = test_store();
        run(Arc::clone(&store), Command::Person {
            action: PersonAction::Add {
                slug: "alice".into(),
                name: Some("Alice Smith".into()),
                meta: vec![],
            },
        });

        let renamed = json(&run(Arc::clone(&store), Command::Person {
            action: PersonAction::Rename {
                old_slug: "alice".into(),
                new_slug: "alice-smith".into(),
            },
        }));
        assert_eq!(renamed["slug"], "alice-smith");
        assert_eq!(renamed["metadata"]["name"], "Alice Smith");

        // old slug gone
        assert!(store.get_person("alice").is_err());
    }

    #[test]
    fn person_update_metadata() {
        let (store, _dir) = test_store();
        run(Arc::clone(&store), Command::Person {
            action: PersonAction::Add {
                slug: "bob".into(),
                name: None,
                meta: vec![],
            },
        });
        let updated = json(&run(Arc::clone(&store), Command::Person {
            action: PersonAction::Update {
                slug: "bob".into(),
                meta: vec![("role".into(), "engineer".into())],
                pinned: None,
                archived: None,
            },
        }));
        assert_eq!(updated["metadata"]["role"], "engineer");
    }

    // ── Tags ──────────────────────────────────────────────────────────────────

    #[test]
    fn tag_add_list_delete() {
        let (store, _dir) = test_store();
        run(Arc::clone(&store), Command::Tag {
            action: TagAction::Add { slug: "work".into() },
        });
        run(Arc::clone(&store), Command::Tag {
            action: TagAction::Add { slug: "personal".into() },
        });

        let list = json(&run(Arc::clone(&store), Command::Tag {
            action: TagAction::List,
        }));
        let slugs: Vec<&str> = list.as_array().expect("tag list should be an array")
            .iter()
            .map(|t| t["slug"].as_str().expect("tag missing 'slug'"))
            .collect();
        assert!(slugs.contains(&"work"));
        assert!(slugs.contains(&"personal"));

        run(Arc::clone(&store), Command::Tag {
            action: TagAction::Delete { slug: "work".into() },
        });
        let list2 = json(&run(Arc::clone(&store), Command::Tag {
            action: TagAction::List,
        }));
        let slugs2: Vec<&str> = list2.as_array().expect("tag list should be an array after delete")
            .iter()
            .map(|t| t["slug"].as_str().expect("tag missing 'slug'"))
            .collect();
        assert!(!slugs2.contains(&"work"));
        assert!(slugs2.contains(&"personal"));
    }

    // ── Agendas ───────────────────────────────────────────────────────────────

    #[test]
    fn agenda_add_and_get_roundtrip() {
        let (store, _dir) = test_store();
        // Person must exist first
        run(Arc::clone(&store), Command::Person {
            action: PersonAction::Add { slug: "carol".into(), name: None, meta: vec![] },
        });

        let created = json(&run(Arc::clone(&store), Command::Agenda {
            action: AgendaAction::Add {
                title: "1:1 with Carol".into(),
                person: "carol".into(),
                date: "2026-05-15".into(),
                body: Some("Discuss roadmap".into()),
                body_stdin: false,
            },
        }));
        assert_eq!(created["title"], "1:1 with Carol");
        assert_eq!(created["person_slug"], "carol");
        assert_eq!(created["date"], "2026-05-15");
        assert_eq!(created["body"], "Discuss roadmap");

        let id = created["id"].as_str().expect("created agenda missing 'id'").to_owned();
        let fetched = json(&run(Arc::clone(&store), Command::Agenda {
            action: AgendaAction::Get { id },
        }));
        assert_eq!(fetched["body"], "Discuss roadmap");
    }

    #[test]
    fn agenda_list_by_person() {
        let (store, _dir) = test_store();
        for slug in &["alice", "bob"] {
            run(Arc::clone(&store), Command::Person {
                action: PersonAction::Add { slug: (*slug).into(), name: None, meta: vec![] },
            });
        }
        run(Arc::clone(&store), Command::Agenda {
            action: AgendaAction::Add {
                title: "Alice meeting".into(),
                person: "alice".into(),
                date: "2026-05-10".into(),
                body: None,
                body_stdin: false,
            },
        });
        run(Arc::clone(&store), Command::Agenda {
            action: AgendaAction::Add {
                title: "Bob meeting".into(),
                person: "bob".into(),
                date: "2026-05-11".into(),
                body: None,
                body_stdin: false,
            },
        });

        let alice_only = json(&run(Arc::clone(&store), Command::Agenda {
            action: AgendaAction::List { person: Some("alice".into()) },
        }));
        let arr = alice_only.as_array().expect("agenda list should be an array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["title"], "Alice meeting");
    }

    // ── Search ────────────────────────────────────────────────────────────────

    #[test]
    fn search_finds_task_by_title() {
        let (store, _dir) = test_store();
        run(Arc::clone(&store), Command::Task {
            action: TaskAction::Add {
                title: "Fix the login bug".into(),
                status: None, priority: None, due: None,
                body: None, body_stdin: false, private: false, pinned: false,
            },
        });
        run(Arc::clone(&store), Command::Task {
            action: TaskAction::Add {
                title: "Buy groceries".into(),
                status: None, priority: None, due: None,
                body: None, body_stdin: false, private: false, pinned: false,
            },
        });

        let results = json(&run(Arc::clone(&store), Command::Search {
            query: "login".into(),
            tag: None,
        }));
        let arr = results.as_array().expect("search results should be an array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["kind"], "task");
        assert_eq!(arr[0]["title"], "Fix the login bug");
    }
}
