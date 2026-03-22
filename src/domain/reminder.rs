use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::is_false;
use super::refs::Refs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reminder {
    pub id: String,
    pub title: String,
    pub remind_at: DateTime<Utc>,

    #[serde(default, skip_serializing_if = "is_false")]
    pub dismissed: bool,

    pub created_at: DateTime<Utc>,

    #[serde(default)]
    pub refs: Refs,
}
