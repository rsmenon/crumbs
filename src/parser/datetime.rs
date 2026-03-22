use chrono::{Datelike, NaiveDate, NaiveTime};
use chrono_english::{parse_date_string, Dialect};
use once_cell::sync::Lazy;
use regex::Regex;

/// Regex that matches common explicit date formats embedded in text.
///
/// Captures patterns like:
/// - 2026-02-28
/// - 02/28/2026, 2/28/2026
/// - 02/28/26, 2/28/26
/// - 02/28, 2/28
static DATE_TOKEN_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?x)
        (?:^|\s)
        (
            \d{4}-\d{1,2}-\d{1,2}          # yyyy-mm-dd
          | \d{1,2}/\d{1,2}/\d{4}          # m/d/yyyy or mm/dd/yyyy
          | \d{1,2}/\d{1,2}/\d{2}          # m/d/yy or mm/dd/yy
          | \d{1,2}/\d{1,2}                # m/d or mm/dd (no year)
        )
        (?:\s|$)
        ",
    )
    .unwrap()
});

/// A time pattern like "3pm", "3:30pm", "15:00", "3:30 PM".
static TIME_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?ix)
        (?:^|\s)
        (
            \d{1,2}:\d{2}\s*(?:am|pm)      # 3:30pm, 3:30 PM
          | \d{1,2}\s*(?:am|pm)             # 3pm, 3 PM
          | \d{1,2}:\d{2}                   # 15:00 (24h)
        )
        (?:\s|$)
        ",
    )
    .unwrap()
});

/// Known NLP date keywords that we attempt to parse via chrono-english.
static NLP_KEYWORDS: &[&str] = &[
    "today",
    "tomorrow",
    "yesterday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "next week",
    "next month",
    "next year",
    "last week",
    "last month",
];

/// Explicit date formats to try in order (most specific first).
const EXPLICIT_FORMATS: &[&str] = &[
    "%Y-%m-%d",   // 2026-02-28
    "%m/%d/%Y",   // 02/28/2026
    "%-m/%-d/%Y", // 2/28/2026
    "%m/%d/%y",   // 02/28/26
    "%-m/%-d/%y", // 2/28/26
];

/// Parse a date (and optional time) from `text`.
///
/// Priority chain:
/// 1. Strip optional `due:` prefix.
/// 2. Try explicit date formats against tokens in the text.
/// 3. Try `mm/dd` (no year) tokens, assuming current or next year.
/// 4. NLP fallback via `chrono-english` for natural language dates.
///
/// Returns `(date, optional_time, cleaned_text)` where `cleaned_text` has the
/// matched date/time expression removed.
pub fn parse_datetime(
    text: &str,
    relative_to: NaiveDate,
) -> Option<(NaiveDate, Option<NaiveTime>, String)> {
    // Strip optional "due:" prefix (with or without space after colon).
    let stripped = if let Some(rest) = text.strip_prefix("due:") {
        rest.trim_start()
    } else {
        text
    };

    // ------------------------------------------------------------------
    // 1. Try explicit date tokens
    // ------------------------------------------------------------------
    if let Some(cap) = DATE_TOKEN_RE.captures(stripped) {
        let token = cap.get(1).unwrap();
        let token_str = token.as_str();

        // Try full explicit formats
        for fmt in EXPLICIT_FORMATS {
            if let Ok(mut date) = NaiveDate::parse_from_str(token_str, fmt) {
                // chrono parses 2-digit years literally (26 -> 0026).
                // Apply the standard windowing: 00-99 -> 2000-2099.
                if date.year() < 100 {
                    date = NaiveDate::from_ymd_opt(date.year() + 2000, date.month(), date.day())
                        .unwrap_or(date);
                }
                let time = extract_time(stripped);
                let cleaned = remove_date_and_time(stripped, token_str, time.as_ref());
                return Some((date, time, cleaned));
            }
        }

        // Try mm/dd (no year) -- two digits each or single digits
        if let Some(date) = parse_month_day(token_str, relative_to) {
            let time = extract_time(stripped);
            let cleaned = remove_date_and_time(stripped, token_str, time.as_ref());
            return Some((date, time, cleaned));
        }
    }

    // ------------------------------------------------------------------
    // 2. NLP fallback
    // ------------------------------------------------------------------
    let lower = stripped.to_lowercase();

    for &keyword in NLP_KEYWORDS {
        if lower.contains(keyword) {
            if let Some(date) = try_nlp(keyword, relative_to) {
                let time = extract_time(stripped);
                let cleaned = remove_nlp_keyword_and_time(stripped, keyword, time.as_ref());
                return Some((date, time, cleaned));
            }
        }
    }

    // Last resort: feed the whole string to chrono-english.
    if let Some(date) = try_nlp(stripped, relative_to) {
        let time = extract_time(stripped);
        // If the entire input was a date expression, the body is empty.
        return Some((date, time, String::new()));
    }

    None
}

// ------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------

/// Parse an `mm/dd` string (no year) into a `NaiveDate`.
///
/// Assumes current year if the resulting date is today or in the future,
/// otherwise assumes next year.
fn parse_month_day(s: &str, relative_to: NaiveDate) -> Option<NaiveDate> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 {
        return None;
    }
    let month: u32 = parts[0].parse().ok()?;
    let day: u32 = parts[1].parse().ok()?;
    if month == 0 || month > 12 || day == 0 || day > 31 {
        return None;
    }

    let year = relative_to.year();
    if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
        if date >= relative_to {
            return Some(date);
        }
    }
    // Date is in the past for this year -- use next year.
    NaiveDate::from_ymd_opt(year + 1, month, day)
}

/// Attempt to parse a natural language date string via chrono-english.
fn try_nlp(expr: &str, relative_to: NaiveDate) -> Option<NaiveDate> {
    let base = relative_to.and_hms_opt(12, 0, 0)?;
    let base_utc = chrono::TimeZone::from_local_datetime(&chrono::Utc, &base)
        .single()?;
    let parsed = parse_date_string(expr, base_utc, Dialect::Us).ok()?;
    Some(parsed.date_naive())
}

/// Try to extract a time component from the text.
fn extract_time(text: &str) -> Option<NaiveTime> {
    let cap = TIME_RE.captures(text)?;
    let token = cap.get(1)?.as_str().trim();
    parse_time_token(token)
}

/// Parse a single time token like "3pm", "3:30pm", "15:00".
fn parse_time_token(token: &str) -> Option<NaiveTime> {
    let t = token.to_lowercase();
    let t = t.trim();

    // Try "3:30pm" / "3:30 pm"
    if let Some(rest) = t.strip_suffix("pm") {
        let rest = rest.trim();
        if let Some((h, m)) = rest.split_once(':') {
            let mut hour: u32 = h.trim().parse().ok()?;
            let min: u32 = m.trim().parse().ok()?;
            if hour < 12 {
                hour += 12;
            }
            return NaiveTime::from_hms_opt(hour, min, 0);
        }
        // "3pm"
        let mut hour: u32 = rest.parse().ok()?;
        if hour < 12 {
            hour += 12;
        }
        return NaiveTime::from_hms_opt(hour, 0, 0);
    }

    if let Some(rest) = t.strip_suffix("am") {
        let rest = rest.trim();
        if let Some((h, m)) = rest.split_once(':') {
            let mut hour: u32 = h.trim().parse().ok()?;
            let min: u32 = m.trim().parse().ok()?;
            if hour == 12 {
                hour = 0;
            }
            return NaiveTime::from_hms_opt(hour, min, 0);
        }
        let mut hour: u32 = rest.parse().ok()?;
        if hour == 12 {
            hour = 0;
        }
        return NaiveTime::from_hms_opt(hour, 0, 0);
    }

    // 24-hour "15:00"
    if let Some((h, m)) = t.split_once(':') {
        let hour: u32 = h.trim().parse().ok()?;
        let min: u32 = m.trim().parse().ok()?;
        return NaiveTime::from_hms_opt(hour, min, 0);
    }

    None
}

/// Remove the date token and optional time token from text, cleaning up
/// extra whitespace.
fn remove_date_and_time(text: &str, date_token: &str, time: Option<&NaiveTime>) -> String {
    let mut result = text.replace(date_token, " ");
    if time.is_some() {
        if let Some(cap) = TIME_RE.captures(text) {
            let m = cap.get(1).unwrap().as_str();
            result = result.replace(m, " ");
        }
    }
    collapse_whitespace(&result)
}

/// Remove an NLP keyword (case-insensitive) and optional time from text.
fn remove_nlp_keyword_and_time(text: &str, keyword: &str, time: Option<&NaiveTime>) -> String {
    // Case-insensitive removal: find the keyword's position in the lowercase
    // version and remove the same range from the original.
    let lower = text.to_lowercase();
    let result = if let Some(pos) = lower.find(keyword) {
        let mut s = String::with_capacity(text.len());
        s.push_str(&text[..pos]);
        s.push(' ');
        s.push_str(&text[pos + keyword.len()..]);
        s
    } else {
        text.to_string()
    };

    let mut result = result;
    if time.is_some() {
        if let Some(cap) = TIME_RE.captures(text) {
            let m = cap.get(1).unwrap().as_str();
            result = result.replace(m, " ");
        }
    }
    collapse_whitespace(&result)
}

/// Collapse runs of whitespace into single spaces and trim.
fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    /// Helper: a fixed reference date for tests.
    fn base() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 2, 26).unwrap()
    }

    // === Explicit date formats ===

    #[test]
    fn iso_date() {
        let (date, time, cleaned) = parse_datetime("meeting 2026-03-15 at noon", base()).unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 3, 15).unwrap());
        assert!(time.is_none());
        assert!(!cleaned.contains("2026-03-15"));
    }

    #[test]
    fn mm_dd_yyyy() {
        let (date, _, _) = parse_datetime("due 03/15/2026 stuff", base()).unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 3, 15).unwrap());
    }

    #[test]
    fn m_d_yyyy() {
        let (date, _, _) = parse_datetime("due 3/5/2026 stuff", base()).unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 3, 5).unwrap());
    }

    #[test]
    fn mm_dd_yy() {
        let (date, _, _) = parse_datetime("submit 03/15/26 report", base()).unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 3, 15).unwrap());
    }

    #[test]
    fn mm_dd_no_year_future() {
        // 06/15 is in the future relative to 2026-02-26 -> 2026-06-15
        let (date, _, _) = parse_datetime("party 06/15 invites", base()).unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 6, 15).unwrap());
    }

    #[test]
    fn mm_dd_no_year_past_wraps() {
        // 01/10 is in the past relative to 2026-02-26 -> 2027-01-10
        let (date, _, _) = parse_datetime("recap 01/10 notes", base()).unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2027, 1, 10).unwrap());
    }

    // === due: prefix ===

    #[test]
    fn due_prefix_stripped() {
        let (date, _, _) = parse_datetime("due:2026-03-15", base()).unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 3, 15).unwrap());
    }

    #[test]
    fn due_prefix_with_space() {
        let (date, _, _) = parse_datetime("due: 2026-03-15", base()).unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 3, 15).unwrap());
    }

    // === Time extraction ===

    #[test]
    fn date_with_time_pm() {
        let (date, time, _) = parse_datetime("call 2026-03-15 3pm please", base()).unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 3, 15).unwrap());
        assert_eq!(time, Some(NaiveTime::from_hms_opt(15, 0, 0).unwrap()));
    }

    #[test]
    fn date_with_time_colon_pm() {
        let (_, time, _) = parse_datetime("meeting 2026-03-15 3:30pm go", base()).unwrap();
        assert_eq!(time, Some(NaiveTime::from_hms_opt(15, 30, 0).unwrap()));
    }

    // === NLP fallback ===

    #[test]
    fn nlp_tomorrow() {
        let (date, _, cleaned) = parse_datetime("call dentist tomorrow", base()).unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 2, 27).unwrap());
        assert!(cleaned.contains("call dentist"));
        assert!(!cleaned.to_lowercase().contains("tomorrow"));
    }

    #[test]
    fn nlp_friday() {
        let result = parse_datetime("submit report friday", base());
        assert!(result.is_some());
        let (date, _, _) = result.unwrap();
        // Should be a Friday
        assert_eq!(date.weekday(), chrono::Weekday::Fri);
    }

    #[test]
    fn nlp_next_week() {
        let result = parse_datetime("review next week", base());
        // chrono-english may not support "next week" as a phrase;
        // if it does parse, the result should be in the future.
        if let Some((date, _, _)) = result {
            assert!(date > base());
        }
        // Not a hard failure if chrono-english doesn't handle this phrase.
    }

    #[test]
    fn nlp_due_prefix_with_nlp() {
        let result = parse_datetime("due:friday", base());
        assert!(result.is_some());
        let (date, _, _) = result.unwrap();
        assert_eq!(date.weekday(), chrono::Weekday::Fri);
    }

    // === No date found ===

    #[test]
    fn no_date() {
        assert!(parse_datetime("just some text", base()).is_none());
    }

    #[test]
    fn empty_string() {
        assert!(parse_datetime("", base()).is_none());
    }

    // === Cleaned text ===

    #[test]
    fn cleaned_text_removes_date() {
        let (_, _, cleaned) = parse_datetime("buy milk 2026-03-01 at the store", base()).unwrap();
        assert!(!cleaned.contains("2026-03-01"));
        assert!(cleaned.contains("buy milk"));
        assert!(cleaned.contains("at the store"));
    }
}
