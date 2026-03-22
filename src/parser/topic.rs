use once_cell::sync::Lazy;
use regex::Regex;

/// Regex to match #topics.
///
/// Matches `#` preceded by start-of-string or a whitespace / punctuation
/// character, followed by at least one ASCII letter and then optional
/// alphanumeric / hyphen / underscore characters.
static TOPIC_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?:^|[\s,;:!?.()\[\]{}"'])#([A-Za-z][A-Za-z0-9_-]*)"#).unwrap()
});

/// Extract #topics from `text`, returning lowercase slugs.
///
/// Results are deduplicated while preserving first-seen order.
pub fn extract_topics(text: &str) -> Vec<String> {
    let mut seen = Vec::new();
    for cap in TOPIC_RE.captures_iter(text) {
        let slug = cap[1].to_lowercase();
        if !seen.contains(&slug) {
            seen.push(slug);
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string() {
        assert!(extract_topics("").is_empty());
    }

    #[test]
    fn no_topics() {
        assert!(extract_topics("nothing here").is_empty());
    }

    #[test]
    fn single_topic() {
        assert_eq!(extract_topics("check #rust docs"), vec!["rust"]);
    }

    #[test]
    fn multiple_topics() {
        assert_eq!(
            extract_topics("#rust and #tui frameworks"),
            vec!["rust", "tui"]
        );
    }

    #[test]
    fn deduplication_preserves_first_order() {
        assert_eq!(
            extract_topics("#Rust is great, #go is fine, #rust again"),
            vec!["rust", "go"]
        );
    }

    #[test]
    fn topic_at_start() {
        assert_eq!(extract_topics("#rsm project kickoff"), vec!["rsm"]);
    }

    #[test]
    fn topic_after_punctuation() {
        assert_eq!(
            extract_topics("tags: #work,#personal (#urgent)"),
            vec!["work", "personal", "urgent"]
        );
    }

    #[test]
    fn topic_with_hyphens_and_underscores() {
        assert_eq!(
            extract_topics("#my-project and #side_project"),
            vec!["my-project", "side_project"]
        );
    }

    #[test]
    fn topic_lowercased() {
        assert_eq!(extract_topics("#Rust #GO"), vec!["rust", "go"]);
    }

    #[test]
    fn numeric_hash_not_a_topic() {
        // #123 should not match because the first char after # is a digit.
        assert!(extract_topics("#123").is_empty());
    }

    #[test]
    fn topic_after_newline() {
        assert_eq!(
            extract_topics("first line\n#second on new line"),
            vec!["second"]
        );
    }

    #[test]
    fn mixed_mentions_and_topics() {
        // Only topics should be extracted by this function.
        let topics = extract_topics("@alice works on #rsm with @bob (#ratatui)");
        assert_eq!(topics, vec!["rsm", "ratatui"]);
    }
}
