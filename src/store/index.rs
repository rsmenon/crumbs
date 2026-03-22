use std::collections::{HashMap, HashSet};

use crate::domain::EntityRef;

#[cfg(test)]
use crate::domain::EntityKind;

use super::fts::FtsIndex;

/// In-memory index over all entities in the store.
///
/// Provides fast lookup by id, date, person, topic, status, and type.
/// Also contains the full-text search index.
#[derive(Default)]
pub struct Index {
    /// entity id -> EntityRef
    pub by_id: HashMap<String, EntityRef>,
    /// date string (YYYY-MM-DD) -> set of entity ids
    pub by_date: HashMap<String, HashSet<String>>,
    /// person slug -> set of entity ids
    pub by_person: HashMap<String, HashSet<String>>,
    /// topic slug -> set of entity ids
    pub by_topic: HashMap<String, HashSet<String>>,
    /// status string (e.g. "backlog") -> set of entity ids
    pub by_status: HashMap<String, HashSet<String>>,
    /// entity type string (e.g. "task") -> set of entity ids
    pub by_type: HashMap<String, HashSet<String>>,
    /// Full-text search inverted index
    pub fts: FtsIndex,
}

impl Index {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.by_id.clear();
        self.by_date.clear();
        self.by_person.clear();
        self.by_topic.clear();
        self.by_status.clear();
        self.by_type.clear();
        self.fts.clear();
    }

    /// Add an entity to the index.
    ///
    /// - `entity_ref`: the kind + id reference for this entity
    /// - `id`: unique identifier (ULID or slug)
    /// - `dates`: date strings (YYYY-MM-DD) associated with this entity (due date, created_at, etc.)
    /// - `people`: person slugs referenced by this entity
    /// - `topics`: topic slugs referenced by this entity
    /// - `status`: optional status string (for tasks)
    /// - `entity_type`: type string used as the by_type key (e.g. "task", "note")
    /// - `search_text`: concatenated text for full-text search indexing
    pub fn add(
        &mut self,
        entity_ref: EntityRef,
        id: &str,
        dates: &[String],
        people: &[String],
        topics: &[String],
        status: Option<&str>,
        entity_type: &str,
        search_text: &str,
    ) {
        // by_id
        self.by_id.insert(id.to_string(), entity_ref);

        // by_date
        for date in dates {
            self.by_date.entry(date.clone()).or_default().insert(id.to_string());
        }

        // by_person
        for person in people {
            self.by_person.entry(person.clone()).or_default().insert(id.to_string());
        }

        // by_topic
        for topic in topics {
            self.by_topic.entry(topic.clone()).or_default().insert(id.to_string());
        }

        // by_status
        if let Some(s) = status {
            self.by_status.entry(s.to_string()).or_default().insert(id.to_string());
        }

        // by_type
        self.by_type.entry(entity_type.to_string()).or_default().insert(id.to_string());

        // FTS
        let tokens = FtsIndex::tokenize(search_text);
        self.fts.add(id, &tokens, search_text);
    }

    /// Remove an entity from all index posting lists and the FTS index.
    pub fn remove(&mut self, id: &str) {
        self.by_id.remove(id);
        for set in self.by_date.values_mut() { set.remove(id); }
        for set in self.by_person.values_mut() { set.remove(id); }
        for set in self.by_topic.values_mut() { set.remove(id); }
        for set in self.by_status.values_mut() { set.remove(id); }
        for set in self.by_type.values_mut() { set.remove(id); }
        self.by_date.retain(|_, s| !s.is_empty());
        self.by_person.retain(|_, s| !s.is_empty());
        self.by_topic.retain(|_, s| !s.is_empty());
        self.by_status.retain(|_, s| !s.is_empty());
        self.by_type.retain(|_, s| !s.is_empty());
        self.fts.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ref(kind: EntityKind, id: &str) -> EntityRef {
        EntityRef {
            kind,
            id: id.to_string(),
        }
    }

    #[test]
    fn add_indexes_by_id() {
        let mut idx = Index::new();
        idx.add(
            make_ref(EntityKind::Task, "abc"),
            "abc",
            &[],
            &[],
            &[],
            None,
            "task",
            "some text",
        );
        assert!(idx.by_id.contains_key("abc"));
    }

    #[test]
    fn add_indexes_by_date() {
        let mut idx = Index::new();
        idx.add(
            make_ref(EntityKind::Task, "abc"),
            "abc",
            &["2026-01-15".to_string()],
            &[],
            &[],
            None,
            "task",
            "",
        );
        assert!(idx.by_date.get("2026-01-15").unwrap().contains("abc"));
    }

    #[test]
    fn add_indexes_by_person_and_topic() {
        let mut idx = Index::new();
        idx.add(
            make_ref(EntityKind::Task, "abc"),
            "abc",
            &[],
            &["alice".to_string()],
            &["rust".to_string()],
            None,
            "task",
            "",
        );
        assert!(idx.by_person.get("alice").unwrap().contains("abc"));
        assert!(idx.by_topic.get("rust").unwrap().contains("abc"));
    }

    #[test]
    fn add_indexes_by_status_and_type() {
        let mut idx = Index::new();
        idx.add(
            make_ref(EntityKind::Task, "abc"),
            "abc",
            &[],
            &[],
            &[],
            Some("todo"),
            "task",
            "",
        );
        assert!(idx.by_status.get("todo").unwrap().contains("abc"));
        assert!(idx.by_type.get("task").unwrap().contains("abc"));
    }

    #[test]
    fn add_indexes_fts() {
        let mut idx = Index::new();
        idx.add(
            make_ref(EntityKind::Task, "abc"),
            "abc",
            &[],
            &[],
            &[],
            None,
            "task",
            "buy groceries today",
        );
        assert!(!idx.fts.fuzzy_search("groceries").is_empty());
    }

    #[test]
    fn clear_empties_everything() {
        let mut idx = Index::new();
        idx.add(
            make_ref(EntityKind::Task, "abc"),
            "abc",
            &["2026-01-15".to_string()],
            &["alice".to_string()],
            &["rust".to_string()],
            Some("todo"),
            "task",
            "hello world",
        );
        idx.clear();
        assert!(idx.by_id.is_empty());
        assert!(idx.by_date.is_empty());
        assert!(idx.by_person.is_empty());
        assert!(idx.by_topic.is_empty());
        assert!(idx.by_status.is_empty());
        assert!(idx.by_type.is_empty());
        assert!(idx.fts.fuzzy_search("hello").is_empty());
    }

    #[test]
    fn no_duplicate_ids_in_lists() {
        let mut idx = Index::new();
        // Add the same entity twice with the same date
        idx.add(
            make_ref(EntityKind::Task, "abc"),
            "abc",
            &["2026-01-15".to_string()],
            &["alice".to_string()],
            &[],
            None,
            "task",
            "",
        );
        idx.add(
            make_ref(EntityKind::Task, "abc"),
            "abc",
            &["2026-01-15".to_string()],
            &["alice".to_string()],
            &[],
            None,
            "task",
            "",
        );
        assert_eq!(idx.by_date.get("2026-01-15").unwrap().len(), 1);
        assert_eq!(idx.by_person.get("alice").unwrap().len(), 1);
        assert_eq!(idx.by_type.get("task").unwrap().len(), 1);
    }
}
