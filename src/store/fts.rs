use std::collections::{HashMap, HashSet};

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// Inverted full-text search index.
///
/// Tokens are mapped to the set of entity IDs that contain them.
/// Search intersects posting lists so that all query terms must match.
pub struct FtsIndex {
    /// token -> set of entity IDs
    terms: HashMap<String, HashSet<String>>,
    /// entity_id -> full search text (for fuzzy matching)
    texts: HashMap<String, String>,
}

impl FtsIndex {
    pub fn new() -> Self {
        Self {
            terms: HashMap::new(),
            texts: HashMap::new(),
        }
    }
}

impl Default for FtsIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl FtsIndex {

    pub fn clear(&mut self) {
        self.terms.clear();
        self.texts.clear();
    }

    /// Tokenize `text`: lowercase, split on non-alphanumeric, keep tokens of length >= 2.
    pub fn tokenize(text: &str) -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| s.len() >= 2)
            .map(String::from)
            .collect()
    }

    /// Index an entity. Each token in `tokens` gets the `id` added to its posting list.
    /// The `full_text` is stored for fuzzy matching.
    pub fn add(&mut self, id: &str, tokens: &[String], full_text: &str) {
        let id = id.to_string();
        for token in tokens {
            self.terms.entry(token.clone()).or_default().insert(id.clone());
        }
        self.texts.insert(id, full_text.to_string());
    }

    /// Remove an entity from the index. Re-tokenizes the stored text to find
    /// which posting lists to update, then prunes any that become empty.
    pub fn remove(&mut self, id: &str) {
        if let Some(text) = self.texts.remove(id) {
            for token in Self::tokenize(&text) {
                if let Some(posting) = self.terms.get_mut(&token) {
                    posting.remove(id);
                }
            }
            self.terms.retain(|_, ids| !ids.is_empty());
        }
    }

    /// Fuzzy search over stored full texts using nucleo-matcher.
    ///
    /// Returns a list of (entity_id, score) pairs sorted by descending score.
    pub fn fuzzy_search(&self, query: &str) -> Vec<(String, u32)> {
        if query.trim().is_empty() {
            return Vec::new();
        }

        let mut matcher = Matcher::new(Config::DEFAULT);
        let pattern = Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );

        let mut buf = Vec::new();
        let mut results: Vec<(String, u32)> = Vec::new();

        for (id, text) in &self.texts {
            let haystack = Utf32Str::new(text, &mut buf);
            if let Some(score) = pattern.score(haystack, &mut matcher) {
                results.push((id.clone(), score));
            }
        }

        results.sort_by(|a, b| b.1.cmp(&a.1));
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_basic() {
        let tokens = FtsIndex::tokenize("Hello, world! This is a test.");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"this".to_string()));
        assert!(tokens.contains(&"test".to_string()));
        // "is" and "a" are length 2 and 1 respectively
        assert!(tokens.contains(&"is".to_string()));
        assert!(!tokens.contains(&"a".to_string()));
    }

    #[test]
    fn tokenize_strips_punctuation() {
        let tokens = FtsIndex::tokenize("@alice's #rust-lang project");
        assert!(tokens.contains(&"alice".to_string()));
        assert!(tokens.contains(&"rust".to_string()));
        assert!(tokens.contains(&"lang".to_string()));
        assert!(tokens.contains(&"project".to_string()));
    }

    #[test]
    fn clear_empties_index() {
        let mut idx = FtsIndex::new();
        idx.add("1", &FtsIndex::tokenize("hello world"), "hello world");
        assert!(!idx.fuzzy_search("hello").is_empty());
        idx.clear();
        assert!(idx.fuzzy_search("hello").is_empty());
    }

    #[test]
    fn no_duplicates_in_posting_list() {
        let mut idx = FtsIndex::new();
        let tokens = FtsIndex::tokenize("hello hello hello");
        idx.add("1", &tokens, "hello hello hello");
        let list = idx.terms.get("hello").unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn fuzzy_search_finds_partial_match() {
        let mut idx = FtsIndex::new();
        idx.add("1", &FtsIndex::tokenize("buy groceries today"), "buy groceries today");
        idx.add("2", &FtsIndex::tokenize("prototype report"), "prototype report");
        idx.add("3", &FtsIndex::tokenize("meeting notes"), "meeting notes");

        let results = idx.fuzzy_search("proto");
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "2"); // "prototype report" should match
    }

    #[test]
    fn fuzzy_search_ranks_by_relevance() {
        let mut idx = FtsIndex::new();
        idx.add("1", &FtsIndex::tokenize("the prototype is ready"), "the prototype is ready");
        idx.add("2", &FtsIndex::tokenize("proto"), "proto");

        let results = idx.fuzzy_search("proto");
        assert!(results.len() >= 2);
        // Exact match "proto" should score higher than "prototype"
    }

    #[test]
    fn remove_cleans_posting_lists() {
        let mut idx = FtsIndex::new();
        idx.add("1", &FtsIndex::tokenize("buy groceries"), "buy groceries");
        idx.add("2", &FtsIndex::tokenize("buy laptop"), "buy laptop");
        idx.remove("1");
        // "groceries" posting list should be gone entirely
        assert!(!idx.terms.contains_key("groceries"));
        // "buy" still has entity 2
        assert!(idx.terms.get("buy").unwrap().contains("2"));
        // fuzzy search should not find entity 1
        let results = idx.fuzzy_search("groceries");
        assert!(results.iter().all(|(id, _)| id != "1"));
    }

    #[test]
    fn remove_nonexistent_is_noop() {
        let mut idx = FtsIndex::new();
        idx.add("1", &FtsIndex::tokenize("hello world"), "hello world");
        idx.remove("999"); // should not panic
        assert!(idx.terms.contains_key("hello"));
    }

    #[test]
    fn fuzzy_search_empty_query_returns_empty() {
        let mut idx = FtsIndex::new();
        idx.add("1", &FtsIndex::tokenize("hello world"), "hello world");
        assert!(idx.fuzzy_search("").is_empty());
        assert!(idx.fuzzy_search("   ").is_empty());
    }
}
