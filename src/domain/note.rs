use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::is_false;
use super::refs::Refs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    #[serde(default, skip_serializing_if = "is_false")]
    pub private: bool,

    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,

    #[serde(default, skip_serializing_if = "is_false")]
    pub archived: bool,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_dir: String,

    #[serde(default)]
    pub refs: Refs,

    /// Markdown body content. Not serialized to JSON front matter;
    /// stored as the body of the .md file after the YAML front matter.
    #[serde(skip)]
    pub body: String,
}
