use chrono::{Datelike, NaiveDate};

/// Returns a 6x7 grid representing a calendar month.
///
/// Each cell is 0 (empty) or a day number (1-31).
/// Rows are weeks, columns are days of the week (Sun=0 .. Sat=6).
pub fn month_grid(year: i32, month: u32) -> [[u8; 7]; 6] {
    let mut grid = [[0u8; 7]; 6];
    let total = days_in_month(year, month);
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("invalid year/month");
    // chrono Weekday: Mon=0 .. Sun=6.  We want Sun=0 .. Sat=6.
    let start_col = match first.weekday() {
        chrono::Weekday::Sun => 0,
        chrono::Weekday::Mon => 1,
        chrono::Weekday::Tue => 2,
        chrono::Weekday::Wed => 3,
        chrono::Weekday::Thu => 4,
        chrono::Weekday::Fri => 5,
        chrono::Weekday::Sat => 6,
    };

    let mut row = 0;
    let mut col = start_col;
    for day in 1..=total {
        grid[row][col] = day as u8;
        col += 1;
        if col == 7 {
            col = 0;
            row += 1;
        }
    }
    grid
}

/// Returns the number of days in the given month of the given year.
pub fn days_in_month(year: i32, month: u32) -> u32 {
    // The first day of the *next* month minus one day gives the last day
    // of the current month.
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let next_first = NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .expect("invalid year/month for days_in_month");
    let first = NaiveDate::from_ymd_opt(year, month, 1)
        .expect("invalid year/month for days_in_month");
    (next_first - first).num_days() as u32
}

/// Returns the weekday column headers: Su Mo Tu We Th Fr Sa.
pub fn weekday_headers() -> [&'static str; 7] {
    ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_in_january() {
        assert_eq!(days_in_month(2024, 1), 31);
    }

    #[test]
    fn days_in_february_leap_year() {
        assert_eq!(days_in_month(2024, 2), 29);
    }

    #[test]
    fn days_in_february_non_leap_year() {
        assert_eq!(days_in_month(2023, 2), 28);
    }

    #[test]
    fn days_in_april() {
        assert_eq!(days_in_month(2024, 4), 30);
    }

    #[test]
    fn days_in_december() {
        assert_eq!(days_in_month(2024, 12), 31);
    }

    #[test]
    fn weekday_headers_start_with_su() {
        let h = weekday_headers();
        assert_eq!(h[0], "Su");
        assert_eq!(h[6], "Sa");
        assert_eq!(h.len(), 7);
    }

    #[test]
    fn month_grid_january_2024() {
        // Jan 2024 starts on a Monday (col 1).
        let grid = month_grid(2024, 1);
        // First row: Sun is empty, Mon=1
        assert_eq!(grid[0][0], 0); // Sun empty
        assert_eq!(grid[0][1], 1); // Mon = Jan 1
        assert_eq!(grid[0][6], 6); // Sat = Jan 6
        // Last day: Jan 31 is a Wednesday (col 3)
        assert_eq!(grid[4][3], 31);
    }

    #[test]
    fn month_grid_february_2024_leap() {
        // Feb 2024 starts on a Thursday (col 4).
        let grid = month_grid(2024, 2);
        assert_eq!(grid[0][4], 1); // Thu = Feb 1
        // Feb 29 is a Thursday (col 4), row 4
        assert_eq!(grid[4][4], 29);
    }

    #[test]
    fn month_grid_september_2024() {
        // Sep 2024 starts on a Sunday (col 0).
        let grid = month_grid(2024, 9);
        assert_eq!(grid[0][0], 1); // Sun = Sep 1
        // Sep 30 is a Monday (col 1)
        assert_eq!(grid[4][1], 30);
    }

    #[test]
    fn month_grid_has_no_overflow() {
        // Every cell is either 0 or in 1..=31
        for year in [2023, 2024, 2025] {
            for month in 1..=12 {
                let grid = month_grid(year, month);
                for row in &grid {
                    for &cell in row {
                        assert!(cell <= 31, "cell value {} out of range", cell);
                    }
                }
            }
        }
    }

    #[test]
    fn month_grid_contains_all_days() {
        for year in [2023, 2024, 2025] {
            for month in 1..=12 {
                let grid = month_grid(year, month);
                let total = days_in_month(year, month);
                let mut found = vec![false; total as usize + 1];
                for row in &grid {
                    for &cell in row {
                        if cell > 0 {
                            found[cell as usize] = true;
                        }
                    }
                }
                for day in 1..=total {
                    assert!(found[day as usize], "day {} missing in {}-{}", day, year, month);
                }
            }
        }
    }
}
