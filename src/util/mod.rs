pub mod calendar;
pub mod date_format;

/// Move the byte-offset cursor one character backward in a UTF-8 string.
pub fn cursor_prev(s: &str, pos: usize) -> usize {
    s[..pos]
        .char_indices()
        .next_back()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Move the byte-offset cursor one character forward in a UTF-8 string.
pub fn cursor_next(s: &str, pos: usize) -> usize {
    pos + s[pos..].chars().next().map(|c| c.len_utf8()).unwrap_or(0)
}
