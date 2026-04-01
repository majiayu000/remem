use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkStreamStatus {
    Active,
    Paused,
    Completed,
    Abandoned,
}

impl WorkStreamStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Abandoned => "abandoned",
        }
    }

    pub fn from_db(s: &str) -> Self {
        match s {
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "abandoned" => Self::Abandoned,
            _ => Self::Active,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkStream {
    pub id: i64,
    pub project: String,
    pub title: String,
    pub description: Option<String>,
    pub status: WorkStreamStatus,
    pub progress: Option<String>,
    pub next_action: Option<String>,
    pub blockers: Option<String>,
    pub created_at_epoch: i64,
    pub updated_at_epoch: i64,
    pub completed_at_epoch: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ParsedWorkStream {
    pub title: Option<String>,
    pub progress: Option<String>,
    pub next_action: Option<String>,
    pub blockers: Option<String>,
    pub is_completed: bool,
}
