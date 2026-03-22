use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::is_false;
use super::refs::Refs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    pub id: String,
    pub title: String,

    #[serde(default, skip_serializing_if = "is_false")]
    pub done: bool,

    pub created_at: DateTime<Utc>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_at: Option<DateTime<Utc>>,

    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::domain::opt_naive_date")]
    pub due_date: Option<NaiveDate>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_time: Option<String>,

    #[serde(default)]
    pub refs: Refs,
}
