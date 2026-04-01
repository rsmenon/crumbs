/// A simple single-line text input buffer.
///
/// Encapsulates the common `push(c)` / `pop()` / `clear()` pattern
/// used across multiple views and the app's tag filter prompt.
#[derive(Debug, Clone, Default)]
pub struct TextInput {
    buf: String,
}

impl TextInput {
    /// Create an empty input buffer.
    pub fn new() -> Self {
        Self { buf: String::new() }
    }

    /// Push a character onto the end of the buffer.
    pub fn push(&mut self, c: char) {
        self.buf.push(c);
    }

    /// Remove and return the last character, if any.
    pub fn pop(&mut self) -> Option<char> {
        self.buf.pop()
    }

    /// Clear the buffer.
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// Return the current buffer contents as a string slice.
    pub fn value(&self) -> &str {
        &self.buf
    }

    /// Replace the buffer contents with the given string.
    pub fn set(&mut self, s: impl Into<String>) {
        self.buf = s.into();
    }
}

impl std::fmt::Display for TextInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.buf)
    }
}
