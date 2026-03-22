use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::refs::Refs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agenda {
    pub id: String,
    pub title: String,
    /// The person slug this agenda belongs to.
    pub person_slug: String,
    /// The date of the agenda (YYYY-MM-DD format on disk).
    #[serde(with = "crate::domain::naive_date_fmt")]
    pub date: NaiveDate,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Body content stored after YAML front matter in the .md file.
    #[serde(skip)]
    pub body: String,
    #[serde(default)]
    pub refs: Refs,
}
