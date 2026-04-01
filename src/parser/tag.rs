use std::collections::HashSet;

use once_cell::sync::Lazy;
use regex::Regex;

/// Regex to match #tags.
///
/// Matches `#` preceded by start-of-string or a whitespace / punctuation
/// character, followed by at least one ASCII letter and then optional
/// alphanumeric / hyphen / underscore characters.
static TAG_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?:^|[\s,;:!?.()\[\]{}"'])#([A-Za-z][A-Za-z0-9_-]*)"#).unwrap()
});

/// Extract #tags from `text`, returning lowercase slugs.
///
/// Results are deduplicated while preserving first-seen order.
pub fn extract_tags(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for cap in TAG_RE.captures_iter(text) {
        let slug = cap[1].to_lowercase();
        if seen.insert(slug.clone()) {
            result.push(slug);
        }
    }
    result
}

/// Backward-compatible alias.
pub fn extract_topics(text: &str) -> Vec<String> {
    extract_tags(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string() {
        assert!(extract_tags("").is_empty());
    }

    #[test]
    fn no_tags() {
        assert!(extract_tags("nothing here").is_empty());
    }

    #[test]
    fn single_tag() {
        assert_eq!(extract_tags("check #rust docs"), vec!["rust"]);
    }

    #[test]
    fn multiple_tags() {
        assert_eq!(
            extract_tags("#rust and #tui frameworks"),
            vec!["rust", "tui"]
        );
    }

    #[test]
    fn deduplication_preserves_first_order() {
        assert_eq!(
            extract_tags("#Rust is great, #go is fine, #rust again"),
            vec!["rust", "go"]
        );
    }

    #[test]
    fn tag_at_start() {
        assert_eq!(extract_tags("#rsm project kickoff"), vec!["rsm"]);
    }

    #[test]
    fn tag_after_punctuation() {
        assert_eq!(
            extract_tags("tags: #work,#personal (#urgent)"),
            vec!["work", "personal", "urgent"]
        );
    }

    #[test]
    fn tag_with_hyphens_and_underscores() {
        assert_eq!(
            extract_tags("#my-project and #side_project"),
            vec!["my-project", "side_project"]
        );
    }

    #[test]
    fn tag_lowercased() {
        assert_eq!(extract_tags("#Rust #GO"), vec!["rust", "go"]);
    }

    #[test]
    fn numeric_hash_not_a_tag() {
        // #123 should not match because the first char after # is a digit.
        assert!(extract_tags("#123").is_empty());
    }

    #[test]
    fn tag_after_newline() {
        assert_eq!(
            extract_tags("first line\n#second on new line"),
            vec!["second"]
        );
    }

    #[test]
    fn mixed_mentions_and_tags() {
        // Only tags should be extracted by this function.
        let tags = extract_tags("@alice works on #rsm with @bob (#ratatui)");
        assert_eq!(tags, vec!["rsm", "ratatui"]);
    }
}
