mod id;
mod refs;
mod task;
mod note;
mod person;
mod tag;
mod agenda;

/// Helper used by `#[serde(skip_serializing_if)]` across domain types.
pub(crate) fn is_false(b: &bool) -> bool {
    !*b
}

/// Serde helper for `Option<NaiveDate>` stored as `"YYYY-MM-DD"` strings on disk.
pub(crate) mod opt_naive_date {
    use chrono::NaiveDate;
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(date: &Option<NaiveDate>, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer {
        match date {
            Some(d) => serializer.serialize_str(&d.format("%Y-%m-%d").to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<NaiveDate>, D::Error>
    where D: Deserializer<'de> {
        let opt: Option<String> = Option::deserialize(deserializer)?;
        match opt {
            None => Ok(None),
            Some(s) if s.is_empty() => Ok(None),
            Some(s) => NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                .map(Some)
                .map_err(serde::de::Error::custom),
        }
    }
}

/// Serde helper for required `NaiveDate` fields stored as `"YYYY-MM-DD"` strings on disk.
pub(crate) mod naive_date_fmt {
    use chrono::NaiveDate;
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(date: &NaiveDate, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer {
        serializer.serialize_str(&date.format("%Y-%m-%d").to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<NaiveDate, D::Error>
    where D: Deserializer<'de> {
        let s = String::deserialize(deserializer)?;
        NaiveDate::parse_from_str(&s, "%Y-%m-%d").map_err(serde::de::Error::custom)
    }
}

pub use id::new_id;
pub use refs::{EntityKind, EntityRef, Refs};
pub use task::{Priority, StatusChange, Task, TaskStatus};
pub use note::Note;
pub use person::Person;
pub use tag::Tag;
pub use agenda::Agenda;
