use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Simplified tag entity. Just a slug registry.
/// Richer fields (display_name, aliases, description, metadata)
/// can be added later if a tag management view is built.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub slug: String,
    pub created_at: DateTime<Utc>,
}
