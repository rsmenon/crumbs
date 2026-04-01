use std::collections::HashSet;

use once_cell::sync::Lazy;
use regex::Regex;

/// Regex to match @mentions.
///
/// Matches `@` preceded by start-of-string or a whitespace / punctuation
/// character, followed by at least one ASCII letter and then optional
/// alphanumeric / hyphen / underscore characters.
static MENTION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?:^|[\s,;:!?.()\[\]{}"'])@([A-Za-z][A-Za-z0-9_-]*)"#).unwrap()
});

/// Extract @mentions from `text`, returning lowercase slugs.
///
/// Results are deduplicated while preserving first-seen order.
pub fn extract_mentions(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for cap in MENTION_RE.captures_iter(text) {
        let slug = cap[1].to_lowercase();
        if seen.insert(slug.clone()) {
            result.push(slug);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string() {
        assert!(extract_mentions("").is_empty());
    }

    #[test]
    fn no_mentions() {
        assert!(extract_mentions("hello world").is_empty());
    }

    #[test]
    fn single_mention() {
        assert_eq!(extract_mentions("hello @alice"), vec!["alice"]);
    }

    #[test]
    fn multiple_mentions() {
        assert_eq!(
            extract_mentions("@alice and @bob go to lunch"),
            vec!["alice", "bob"]
        );
    }

    #[test]
    fn deduplication_preserves_first_order() {
        assert_eq!(
            extract_mentions("@Alice then @bob then @alice again"),
            vec!["alice", "bob"]
        );
    }

    #[test]
    fn mention_at_start() {
        assert_eq!(extract_mentions("@charlie says hi"), vec!["charlie"]);
    }

    #[test]
    fn mention_after_punctuation() {
        assert_eq!(
            extract_mentions("hello,@alice and (@bob) and \"@carol\""),
            vec!["alice", "bob", "carol"]
        );
    }

    #[test]
    fn mention_with_hyphens_and_underscores() {
        assert_eq!(
            extract_mentions("cc @mary-jane and @joe_bob"),
            vec!["mary-jane", "joe_bob"]
        );
    }

    #[test]
    fn email_not_a_mention() {
        // The `@` in an email is preceded by a letter, not whitespace/punctuation,
        // so it should NOT match.
        assert!(extract_mentions("user@example.com").is_empty());
    }

    #[test]
    fn mention_lowercased() {
        assert_eq!(extract_mentions("@Alice @BOB"), vec!["alice", "bob"]);
    }

    #[test]
    fn mention_must_start_with_letter() {
        // @123 should not match because the first char after @ is a digit.
        assert!(extract_mentions("@123").is_empty());
    }

    #[test]
    fn mention_after_newline() {
        assert_eq!(
            extract_mentions("line one\n@dave on line two"),
            vec!["dave"]
        );
    }
}
