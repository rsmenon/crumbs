use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::is_false;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person {
    pub slug: String,

    pub created_at: DateTime<Utc>,

    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,

    #[serde(default, skip_serializing_if = "is_false")]
    pub archived: bool,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl Person {
    /// Display name: uses metadata "name" key if present, otherwise @slug.
    pub fn display_name(&self) -> String {
        if let Some(name) = self.metadata.get("name") {
            if !name.is_empty() {
                return format!("@{}", name);
            }
        }
        format!("@{}", self.slug)
    }
}
