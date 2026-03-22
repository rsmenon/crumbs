use chrono::{Datelike, Local, NaiveDate};

/// Formats a date as "Jan 02" for the current year, or "Jan 02, 24" for
/// other years (abbreviated 2-digit year).
pub fn format_date(date: &NaiveDate) -> String {
    let current_year = Local::now().date_naive().year();
    if date.year() == current_year {
        date.format("%b %d").to_string()
    } else {
        date.format("%b %d, %y").to_string()
    }
}

/// Parse a "YYYY-MM-DD" date string and format it with [`format_date`].
/// Returns the original string unchanged if parsing fails.
pub fn format_date_str(date_str: &str) -> String {
    if let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
        format_date(&date)
    } else {
        date_str.to_string()
    }
}

/// Format a `DateTime<Utc>` for display by first converting to local time
/// and then applying year-aware formatting.
pub fn format_utc_date(dt: &chrono::DateTime<chrono::Utc>) -> String {
    let local: chrono::DateTime<Local> = (*dt).into();
    let date = local.date_naive();
    format_date(&date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn format_date_current_year_no_year_suffix() {
        let current_year = Local::now().date_naive().year();
        let d = NaiveDate::from_ymd_opt(current_year, 6, 15).unwrap();
        assert_eq!(format_date(&d), "Jun 15");
    }

    #[test]
    fn format_date_different_year_shows_year() {
        // 2024 is not the current year (tests run in 2026)
        let d = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        assert_eq!(format_date(&d), "Jan 01, 24");
    }

    #[test]
    fn format_date_dec_25_different_year() {
        let d = NaiveDate::from_ymd_opt(2024, 12, 25).unwrap();
        assert_eq!(format_date(&d), "Dec 25, 24");
    }

    #[test]
    fn format_date_single_digit_day() {
        let current_year = Local::now().date_naive().year();
        let d = NaiveDate::from_ymd_opt(current_year, 3, 5).unwrap();
        assert_eq!(format_date(&d), "Mar 05");
    }

    #[test]
    fn format_date_str_valid() {
        let result = format_date_str("2024-01-01");
        assert_eq!(result, "Jan 01, 24");
    }

    #[test]
    fn format_date_str_invalid() {
        let result = format_date_str("not-a-date");
        assert_eq!(result, "not-a-date");
    }

}
