use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::is_false;
use super::refs::Refs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Priority {
    #[default]
    None,
    Low,
    Medium,
    High,
}

impl Priority {
    pub fn label(&self) -> &'static str {
        match self {
            Priority::None => "",
            Priority::Low => "Low",
            Priority::Medium => "Medium",
            Priority::High => "High",
        }
    }

    /// All non-None priority values, in order.
    pub const OPTIONS: [Priority; 4] = [Priority::None, Priority::Low, Priority::Medium, Priority::High];

    /// Cycle to the next priority (wraps around).
    pub fn next(self) -> Priority {
        match self {
            Priority::None => Priority::Low,
            Priority::Low => Priority::Medium,
            Priority::Medium => Priority::High,
            Priority::High => Priority::None,
        }
    }

    /// True when the priority is not set.
    pub fn is_none(&self) -> bool {
        *self == Priority::None
    }
}

impl std::str::FromStr for Priority {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().trim() {
            "" | "none" => Ok(Priority::None),
            "low" => Ok(Priority::Low),
            "medium" | "med" => Ok(Priority::Medium),
            "high" => Ok(Priority::High),
            other => Err(format!("unknown priority: {other}")),
        }
    }
}

impl Serialize for Priority {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.label())
    }
}

impl<'de> Deserialize<'de> for Priority {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(match s.to_lowercase().trim() {
            "" => Priority::None,
            "none" => Priority::None,
            "low" => Priority::Low,
            "medium" | "med" => Priority::Medium,
            "high" => Priority::High,
            _ => Priority::None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Backlog,
    Todo,
    InProgress,
    Blocked,
    Done,
    Archived,
}

impl TaskStatus {
    /// Ordered list of statuses for cycling (Archived is excluded; use the
    /// `archived` flag on Task instead).
    const ORDER: [TaskStatus; 5] = [
        TaskStatus::Backlog,
        TaskStatus::Todo,
        TaskStatus::InProgress,
        TaskStatus::Blocked,
        TaskStatus::Done,
    ];

    /// Numeric index for sorting (matches ORDER).
    pub fn index(&self) -> usize {
        Self::ORDER
            .iter()
            .position(|s| s == self)
            .unwrap_or(Self::ORDER.len())
    }

    /// Cycle to the next status (wraps around).
    /// Archived status cycles to Backlog (start of ORDER).
    pub fn next(&self) -> TaskStatus {
        match Self::ORDER.iter().position(|s| s == self) {
            Some(idx) => Self::ORDER[(idx + 1) % Self::ORDER.len()],
            None => Self::ORDER[0], // Archived → Backlog
        }
    }

    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            TaskStatus::Backlog => "Backlog",
            TaskStatus::Todo => "Todo",
            TaskStatus::InProgress => "Doing",
            TaskStatus::Blocked => "Blocked",
            TaskStatus::Done => "Done",
            TaskStatus::Archived => "Archived",
        }
    }

    /// Snake_case string matching the serde serialization (for front matter).
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Backlog => "backlog",
            TaskStatus::Todo => "todo",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Blocked => "blocked",
            TaskStatus::Done => "done",
            TaskStatus::Archived => "archived",
        }
    }

    /// Parse from a string (case-insensitive, accepts both snake_case and Display forms).
    pub fn from_str_loose(s: &str) -> Option<TaskStatus> {
        match s.to_lowercase().trim() {
            "backlog" => Some(TaskStatus::Backlog),
            "todo" => Some(TaskStatus::Todo),
            "in_progress" | "in progress" | "inprogress" | "doing" => Some(TaskStatus::InProgress),
            "blocked" => Some(TaskStatus::Blocked),
            "done" => Some(TaskStatus::Done),
            "archived" => Some(TaskStatus::Archived),
            _ => None,
        }
    }

    /// Unicode icon for this status.
    pub fn icon(&self) -> &'static str {
        match self {
            TaskStatus::Backlog => "◌",
            TaskStatus::Todo => "○",
            TaskStatus::InProgress => "●",
            TaskStatus::Blocked => "⊘",
            TaskStatus::Done => "✓",
            TaskStatus::Archived => "⊟",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusChange {
    pub status: TaskStatus,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,

    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::domain::opt_naive_date")]
    pub due_date: Option<NaiveDate>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_time: Option<String>,

    #[serde(default, skip_serializing_if = "Priority::is_none")]
    pub priority: Priority,

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

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status_history: Vec<StatusChange>,
}

impl Default for Task {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: super::id::new_id(),
            title: String::new(),
            description: String::new(),
            status: TaskStatus::Backlog,
            created_at: now,
            updated_at: now,
            due_date: None,
            due_time: None,
            priority: Priority::None,
            private: false,
            pinned: false,
            archived: false,
            created_dir: String::new(),
            refs: Refs::default(),
            status_history: Vec::new(),
        }
    }
}
