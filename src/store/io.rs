use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tempfile::NamedTempFile;

use crate::domain::{Agenda, Note};

/// Read and deserialize a JSON file.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let value = serde_json::from_str(&content)
        .with_context(|| format!("parsing JSON from {}", path.display()))?;
    Ok(value)
}

/// Atomically write a value as pretty-printed JSON.
///
/// Writes to a temporary file in the same directory, then renames into place.
/// This avoids partial writes if the process is interrupted.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let dir = path
        .parent()
        .context("path has no parent directory")?;
    fs::create_dir_all(dir)?;

    let mut tmp = NamedTempFile::new_in(dir)
        .with_context(|| format!("creating temp file in {}", dir.display()))?;

    let data = serde_json::to_string_pretty(value)
        .context("serializing to JSON")?;
    tmp.write_all(data.as_bytes())
        .context("writing to temp file")?;
    tmp.flush()?;

    tmp.persist(path)
        .with_context(|| format!("persisting temp file to {}", path.display()))?;

    Ok(())
}

/// Parse a note from a .md file with YAML front matter.
///
/// The file format is:
/// ```text
/// ---
/// id: ...
/// title: ...
/// created_at: ...
/// ...
/// ---
///
/// Markdown body here...
/// ```
///
/// The YAML front matter is deserialized into a Note (minus the `body` field,
/// which is skipped by serde). The body is everything after the closing `---`.
pub fn parse_note(path: &Path) -> Result<Note> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("reading note {}", path.display()))?;

    // Find front matter boundaries
    if !content.starts_with("---\n") && !content.starts_with("---\r\n") {
        anyhow::bail!(
            "note file {} does not start with YAML front matter delimiter",
            path.display()
        );
    }

    // Skip the opening "---\n"
    let after_open = if content.starts_with("---\r\n") {
        &content[5..]
    } else {
        &content[4..]
    };

    // Find the closing "---"
    let close_idx = after_open
        .find("\n---\n")
        .or_else(|| after_open.find("\n---\r\n"))
        .or_else(|| {
            // Handle case where "---" is at the very end with no trailing newline
            if after_open.ends_with("\n---") {
                Some(after_open.len() - 3)
            } else {
                None
            }
        })
        .with_context(|| {
            format!(
                "note file {} is missing closing YAML front matter delimiter",
                path.display()
            )
        })?;

    let fm_text = &after_open[..close_idx];

    // Body starts after the closing "---\n"
    let body_start = close_idx + 1; // skip the \n before ---
    let rest = &after_open[body_start..];
    // Skip the "---\n" itself
    let body = if rest.starts_with("---\r\n") {
        &rest[5..]
    } else if rest.starts_with("---\n") {
        &rest[4..]
    } else if rest.starts_with("---") {
        &rest[3..]
    } else {
        rest
    };

    let mut note: Note = serde_yaml::from_str(fm_text).with_context(|| {
        format!(
            "parsing YAML front matter in {}",
            path.display()
        )
    })?;

    // Trim leading newline from body (common formatting)
    let body = body.strip_prefix('\n').unwrap_or(body);
    let body = body.strip_prefix("\r\n").unwrap_or(body);
    note.body = body.to_string();

    Ok(note)
}

/// Parse an agenda from a .md file with YAML front matter.
pub fn parse_agenda(path: &Path) -> Result<Agenda> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("reading agenda {}", path.display()))?;

    if !content.starts_with("---\n") && !content.starts_with("---\r\n") {
        anyhow::bail!("agenda file {} does not start with YAML front matter delimiter", path.display());
    }

    let after_open = if content.starts_with("---\r\n") { &content[5..] } else { &content[4..] };

    let close_idx = after_open.find("\n---\n")
        .or_else(|| after_open.find("\n---\r\n"))
        .or_else(|| if after_open.ends_with("\n---") { Some(after_open.len() - 3) } else { None })
        .with_context(|| format!("agenda file {} is missing closing YAML front matter delimiter", path.display()))?;

    let fm_text = &after_open[..close_idx];
    let body_start = close_idx + 1;
    let rest = &after_open[body_start..];
    let body = if rest.starts_with("---\r\n") { &rest[5..] }
               else if rest.starts_with("---\n") { &rest[4..] }
               else if rest.starts_with("---")  { &rest[3..] }
               else { rest };
    let body = body.strip_prefix('\n').unwrap_or(body);
    let body = body.strip_prefix("\r\n").unwrap_or(body);

    let mut agenda: Agenda = serde_yaml::from_str(fm_text)
        .with_context(|| format!("parsing YAML front matter in {}", path.display()))?;
    agenda.body = body.to_string();
    Ok(agenda)
}

/// Write an agenda as YAML front matter + markdown body.
pub fn write_agenda(path: &Path, agenda: &Agenda) -> Result<()> {
    let dir = path.parent().context("path has no parent directory")?;
    fs::create_dir_all(dir)?;

    let fm = serde_yaml::to_string(agenda).context("serializing agenda front matter")?;

    let mut tmp = NamedTempFile::new_in(dir)
        .with_context(|| format!("creating temp file in {}", dir.display()))?;

    write!(tmp, "---\n{}---\n\n{}", fm, agenda.body)?;
    tmp.flush()?;
    tmp.persist(path).with_context(|| format!("persisting agenda to {}", path.display()))?;
    Ok(())
}

/// Write a note as YAML front matter + markdown body.
pub fn write_note(path: &Path, note: &Note) -> Result<()> {
    let dir = path
        .parent()
        .context("path has no parent directory")?;
    fs::create_dir_all(dir)?;

    let fm = serde_yaml::to_string(note)
        .context("serializing note front matter")?;

    let mut tmp = NamedTempFile::new_in(dir)
        .with_context(|| format!("creating temp file in {}", dir.display()))?;

    write!(tmp, "---\n{}---\n\n{}", fm, note.body)?;
    tmp.flush()?;

    tmp.persist(path)
        .with_context(|| format!("persisting note to {}", path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Sample {
        name: String,
        value: i32,
    }

    #[test]
    fn roundtrip_json() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.json");

        let sample = Sample {
            name: "hello".into(),
            value: 42,
        };
        write_json(&path, &sample).unwrap();
        let loaded: Sample = read_json(&path).unwrap();
        assert_eq!(loaded, sample);
    }

    #[test]
    fn read_json_file_not_found() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let result: Result<Sample> = read_json(&path);
        assert!(result.is_err());
    }

    #[test]
    fn roundtrip_note() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.md");

        let now = Utc::now();
        let note = Note {
            id: "test-id".to_string(),
            title: "Test Note".to_string(),
            created_at: now,
            updated_at: now,
            private: false,
            pinned: false,
            archived: false,
            created_dir: "/tmp".to_string(),
            refs: crate::domain::Refs::default(),
            body: "Hello, world!\n\nThis is a test note.\n".to_string(),
        };

        write_note(&path, &note).unwrap();
        let loaded = parse_note(&path).unwrap();

        assert_eq!(loaded.id, note.id);
        assert_eq!(loaded.title, note.title);
        assert_eq!(loaded.body, note.body);
        assert_eq!(loaded.private, note.private);
    }

    #[test]
    fn parse_note_missing_front_matter_fails() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.md");
        fs::write(&path, "no front matter here").unwrap();
        assert!(parse_note(&path).is_err());
    }

    #[test]
    fn parse_note_empty_body() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty_body.md");

        let now = Utc::now();
        let note = Note {
            id: "id1".to_string(),
            title: "Empty body".to_string(),
            created_at: now,
            updated_at: now,
            private: false,
            pinned: false,
            archived: false,
            created_dir: String::new(),
            refs: crate::domain::Refs::default(),
            body: String::new(),
        };

        write_note(&path, &note).unwrap();
        let loaded = parse_note(&path).unwrap();
        assert_eq!(loaded.id, "id1");
        // Body may be empty or just whitespace
        assert!(loaded.body.trim().is_empty());
    }
}
