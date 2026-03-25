use chrono::NaiveDate;

use super::datetime::parse_datetime;
use super::mention::extract_mentions;
use super::tag::extract_tags;

/// The classified type of a sink entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkEntryType {
    /// Plain text note (no actionable prefix detected).
    Note,
    /// Begins with "todo:" -- an actionable task.
    Todo,
}

/// A parsed sink entry with extracted metadata.
#[derive(Debug, Clone)]
pub struct ParsedSink {
    /// The classified entry type.
    pub entry_type: SinkEntryType,
    /// Extracted @people references (lowercase slugs, deduplicated).
    pub people: Vec<String>,
    /// Extracted #tag references (lowercase slugs, deduplicated).
    pub tags: Vec<String>,
    /// A parsed date, if one was detected.
    pub datetime: Option<NaiveDate>,
    /// The body text with any type prefix and date expression removed.
    pub body: String,
}

/// Parse raw sink input into a structured [`ParsedSink`].
///
/// Detection priority:
/// 1. "todo:" prefix  -> [`SinkEntryType::Todo`]
/// 2. Otherwise -> [`SinkEntryType::Note`]
///
/// @mentions and #topics are extracted from the body after prefix removal.
/// Dates are still parsed and placed in `datetime` (useful for e.g. `todo: buy milk friday`).
pub fn parse_sink(input: &str) -> ParsedSink {
    let trimmed = input.trim();
    let today = chrono::Local::now().date_naive();

    let (entry_type, body_after_prefix) = classify_and_strip(trimmed);

    // Attempt date parsing on the body (after prefix removal).
    let (datetime, body) = match parse_datetime(&body_after_prefix, today) {
        Some((date, _time, cleaned)) => (Some(date), cleaned),
        None => (None, body_after_prefix.clone()),
    };

    let people = extract_mentions(&body);
    let tags = extract_tags(&body);

    ParsedSink {
        entry_type,
        people,
        tags,
        datetime,
        body: body.trim().to_string(),
    }
}

/// Classify the entry by prefix and return the type plus the remaining body.
fn classify_and_strip(text: &str) -> (SinkEntryType, String) {
    let lower = text.to_lowercase();

    if let Some(rest) = lower.strip_prefix("todo:") {
        let offset = "todo:".len();
        let body = text[offset..].trim_start().to_string();
        let _ = rest; // only used for prefix matching
        return (SinkEntryType::Todo, body);
    }

    (SinkEntryType::Note, text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_note() {
        let p = parse_sink("just a thought");
        assert_eq!(p.entry_type, SinkEntryType::Note);
        assert_eq!(p.body, "just a thought");
        assert!(p.datetime.is_none());
        assert!(p.people.is_empty());
        assert!(p.tags.is_empty());
    }

    #[test]
    fn todo_prefix() {
        let p = parse_sink("todo: buy milk");
        assert_eq!(p.entry_type, SinkEntryType::Todo);
        assert_eq!(p.body, "buy milk");
    }

    #[test]
    fn todo_prefix_case_insensitive() {
        let p = parse_sink("TODO: fix the build");
        assert_eq!(p.entry_type, SinkEntryType::Todo);
        assert_eq!(p.body, "fix the build");
    }

    #[test]
    fn remind_prefix_is_note() {
        let p = parse_sink("remind: call dentist tomorrow");
        assert_eq!(p.entry_type, SinkEntryType::Note);
        assert!(p.datetime.is_some());
        assert!(p.body.contains("call dentist"));
    }

    #[test]
    fn reminder_prefix_is_note() {
        let p = parse_sink("reminder: pay rent 2026-03-01");
        assert_eq!(p.entry_type, SinkEntryType::Note);
        assert!(p.datetime.is_some());
    }

    #[test]
    fn date_stays_note() {
        // No prefix, but a date is present -> still Note (no promotion).
        let p = parse_sink("dentist appointment 2026-04-10");
        assert_eq!(p.entry_type, SinkEntryType::Note);
        assert_eq!(
            p.datetime,
            Some(NaiveDate::from_ymd_opt(2026, 4, 10).unwrap())
        );
    }

    #[test]
    fn mentions_extracted() {
        let p = parse_sink("todo: ask @alice about #project");
        assert_eq!(p.entry_type, SinkEntryType::Todo);
        assert_eq!(p.people, vec!["alice"]);
        assert_eq!(p.tags, vec!["project"]);
    }

    #[test]
    fn multiple_mentions_and_topics() {
        let p = parse_sink("sync with @alice and @bob on #rsm #rust");
        assert_eq!(p.people, vec!["alice", "bob"]);
        assert_eq!(p.tags, vec!["rsm", "rust"]);
    }

    #[test]
    fn empty_input() {
        let p = parse_sink("");
        assert_eq!(p.entry_type, SinkEntryType::Note);
        assert_eq!(p.body, "");
    }

    #[test]
    fn whitespace_only() {
        let p = parse_sink("   ");
        assert_eq!(p.entry_type, SinkEntryType::Note);
        assert_eq!(p.body, "");
    }

    #[test]
    fn todo_with_date_stays_todo() {
        // "todo:" prefix wins the classification even if a date is present.
        let p = parse_sink("todo: submit report 2026-03-15");
        assert_eq!(p.entry_type, SinkEntryType::Todo);
        assert!(p.datetime.is_some());
    }

    #[test]
    fn nlp_date_stays_note() {
        let p = parse_sink("pick up groceries tomorrow");
        // "tomorrow" should be detected by NLP, but entry stays Note
        assert_eq!(p.entry_type, SinkEntryType::Note);
        assert!(p.datetime.is_some());
    }
}
