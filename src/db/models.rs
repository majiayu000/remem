use serde::{Deserialize, Serialize};

pub const OBSERVATION_TYPES: &[&str] = &[
    "bugfix",
    "feature",
    "refactor",
    "discovery",
    "decision",
    "change",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub id: i64,
    pub memory_session_id: String,
    pub r#type: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub narrative: Option<String>,
    pub facts: Option<String>,
    pub concepts: Option<String>,
    pub files_read: Option<String>,
    pub files_modified: Option<String>,
    pub discovery_tokens: Option<i64>,
    pub created_at: String,
    pub created_at_epoch: i64,
    pub project: Option<String>,
    pub status: String,
    pub last_accessed_epoch: Option<i64>,
    /// Original Claude Code session ID (for `claude --resume`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_session_id: Option<String>,
    /// Git branch name at the time of observation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Git short commit SHA at the time of observation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompressedObservationSource {
    pub compressed_observation_id: i64,
    pub source_observation_id: i64,
    pub source_hash: String,
    #[serde(skip_serializing)]
    pub source_snapshot_json: String,
    pub source_created_at_epoch: i64,
    pub compression_session_id: String,
    pub created_at_epoch: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: i64,
    pub memory_session_id: String,
    pub request: Option<String>,
    pub completed: Option<String>,
    pub decisions: Option<String>,
    pub learned: Option<String>,
    pub next_steps: Option<String>,
    pub preferences: Option<String>,
    pub created_at: String,
    pub created_at_epoch: i64,
    pub project: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobType {
    Observation,
    Summary,
    Compress,
    Dream,
}

impl JobType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observation => "observation",
            Self::Summary => "summary",
            Self::Compress => "compress",
            Self::Dream => "dream",
        }
    }

    pub fn from_db(raw: &str) -> anyhow::Result<Self> {
        match raw {
            "observation" => Ok(Self::Observation),
            "summary" => Ok(Self::Summary),
            "compress" => Ok(Self::Compress),
            "dream" => Ok(Self::Dream),
            _ => anyhow::bail!("unknown job_type: {}", raw),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: i64,
    pub host: String,
    pub job_type: JobType,
    pub project: String,
    pub session_id: Option<String>,
    pub payload_json: String,
    pub attempt_count: i64,
    pub max_attempts: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiUsageEvent {
    pub created_at: String,
    pub project: Option<String>,
    pub operation: String,
    pub executor: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cache_read_tokens: i64,
    pub raw_input_tokens: i64,
    pub raw_output_tokens: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: f64,
    pub usage_source: String,
    pub pricing_source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DailyAiUsage {
    pub day: String,
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cache_read_tokens: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WeeklyAiUsage {
    pub week: String,
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cache_read_tokens: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiUsageTotals {
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cache_read_tokens: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiUsageSourceTotals {
    pub usage_source: String,
    pub pricing_source: String,
    pub calls: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiUsageBreakdown {
    pub project: Option<String>,
    pub executor: String,
    pub usage_source: String,
    pub pricing_source: String,
    pub calls: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: f64,
}
