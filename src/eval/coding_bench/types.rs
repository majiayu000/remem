use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::audit_contract::{RememContextAuditSnapshot, RememContextAuditStatus};

#[derive(Debug, Clone)]
pub struct CodingBenchOptions {
    pub fixture_path: String,
    pub runs_per_condition: usize,
    pub json_out: String,
    pub condition: Option<String>,
    pub matrix: String,
    pub task: Option<String>,
    pub task_set: String,
    pub keep_workdirs: bool,
    pub dry_run: bool,
    pub runner: String,
    pub codex_bin: String,
    pub model: String,
    pub provider: Option<String>,
    pub reasoning_effort: String,
    pub ignore_budget: bool,
    pub curator_root: Option<String>,
    pub memory_config: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CodingBenchFixture {
    pub version: u32,
    pub repo: FixtureRepo,
    #[serde(default)]
    pub curated_context: Option<String>,
    pub tasks: Vec<CodingBenchTask>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FixtureRepo {
    pub kind: String,
    pub base_commit: Option<String>,
    pub fixture_revision: Option<String>,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CodingBenchTask {
    pub id: String,
    pub category: String,
    #[serde(default)]
    pub smoke: bool,
    pub prompt: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub forbidden_paths: Vec<String>,
    pub score: ScoreSpec,
    #[serde(default)]
    pub history_episodes: Vec<HistoryEpisode>,
    #[serde(default)]
    pub memories: Vec<SeedMemory>,
    #[serde(default)]
    pub curated_context: Option<String>,
    #[serde(default)]
    pub gold_memory: GoldMemory,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScoreSpec {
    #[serde(default)]
    pub commands: Vec<Vec<String>>,
    #[serde(default)]
    pub hidden_files: BTreeMap<String, String>,
    #[serde(default)]
    pub required_patch_patterns: Vec<String>,
    #[serde(default)]
    pub forbidden_patch_patterns: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryEpisode {
    pub episode_id: String,
    pub reference_time_epoch: i64,
    pub summary: String,
    #[serde(default)]
    pub expected_memory_facts: Vec<String>,
    #[serde(default)]
    pub memories: Vec<SeedMemory>,
    pub raw_events: Vec<RawHistoryEvent>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RawHistoryEvent {
    pub event_id: String,
    pub timestamp_epoch: i64,
    pub role: String,
    pub sanitized_content: String,
    pub tool_name: Option<String>,
    pub sanitized_tool_input: Option<String>,
    pub sanitized_tool_output: Option<String>,
    pub host_boundary: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeedMemory {
    pub title: String,
    pub text: String,
    #[serde(default)]
    pub memory_type: Option<String>,
    #[serde(default)]
    pub topic_key: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GoldMemory {
    #[serde(default)]
    pub required_facts: Vec<String>,
    #[serde(default)]
    pub forbidden_facts: Vec<String>,
    #[serde(default)]
    pub supporting_event_ids: Vec<String>,
}

impl CodingBenchTask {
    pub fn seed_memories(&self) -> Vec<&SeedMemory> {
        self.history_episodes
            .iter()
            .flat_map(|episode| episode.memories.iter())
            .chain(self.memories.iter())
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchCondition {
    NoMemory,
    CuratedFileBudgeted,
    RememE2e,
    #[serde(rename = "remem_seeded_sessionstart")]
    RememSeededSessionStart,
    CuratedFileExpert,
    OracleEvidence,
    RememOracleRetrieval,
    FullHistory,
    RememNoEnrichment,
    RememFtsOnly,
}

impl BenchCondition {
    pub const PRIMARY: [Self; 3] = [Self::NoMemory, Self::CuratedFileBudgeted, Self::RememE2e];
    pub const DIAGNOSTIC: [Self; 7] = [
        Self::RememSeededSessionStart,
        Self::CuratedFileExpert,
        Self::OracleEvidence,
        Self::RememOracleRetrieval,
        Self::FullHistory,
        Self::RememNoEnrichment,
        Self::RememFtsOnly,
    ];
    pub const IMPLEMENTED: [Self; 5] = [
        Self::NoMemory,
        Self::CuratedFileBudgeted,
        Self::RememE2e,
        Self::RememSeededSessionStart,
        Self::CuratedFileExpert,
    ];
    pub const ALL: [Self; 10] = [
        Self::NoMemory,
        Self::CuratedFileBudgeted,
        Self::RememE2e,
        Self::RememSeededSessionStart,
        Self::CuratedFileExpert,
        Self::OracleEvidence,
        Self::RememOracleRetrieval,
        Self::FullHistory,
        Self::RememNoEnrichment,
        Self::RememFtsOnly,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoMemory => "no_memory",
            Self::CuratedFileBudgeted => "curated_file_budgeted",
            Self::RememE2e => "remem_e2e",
            Self::RememSeededSessionStart => "remem_seeded_sessionstart",
            Self::CuratedFileExpert => "curated_file_expert",
            Self::OracleEvidence => "oracle_evidence",
            Self::RememOracleRetrieval => "remem_oracle_retrieval",
            Self::FullHistory => "full_history",
            Self::RememNoEnrichment => "remem_no_enrichment",
            Self::RememFtsOnly => "remem_fts_only",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "no_memory" => Some(Self::NoMemory),
            "curated_file_budgeted" => Some(Self::CuratedFileBudgeted),
            "remem_e2e" => Some(Self::RememE2e),
            "remem_seeded_sessionstart" => Some(Self::RememSeededSessionStart),
            "curated_file_expert" => Some(Self::CuratedFileExpert),
            "oracle_evidence" => Some(Self::OracleEvidence),
            "remem_oracle_retrieval" => Some(Self::RememOracleRetrieval),
            "full_history" => Some(Self::FullHistory),
            "remem_no_enrichment" => Some(Self::RememNoEnrichment),
            "remem_fts_only" => Some(Self::RememFtsOnly),
            _ => None,
        }
    }

    pub const fn supports_live_execution(self) -> bool {
        matches!(
            self,
            Self::NoMemory
                | Self::CuratedFileBudgeted
                | Self::RememE2e
                | Self::RememSeededSessionStart
                | Self::CuratedFileExpert
        )
    }

    pub const fn uses_remem_attribution(self) -> bool {
        matches!(self, Self::RememE2e | Self::RememSeededSessionStart)
    }
}

#[cfg(test)]
mod condition_identity_tests {
    use super::BenchCondition;

    #[test]
    fn seeded_sessionstart_has_distinct_nonlegacy_identity() -> serde_json::Result<()> {
        assert_eq!(
            BenchCondition::parse("remem_seeded_sessionstart"),
            Some(BenchCondition::RememSeededSessionStart)
        );
        assert_eq!(
            BenchCondition::RememSeededSessionStart.as_str(),
            "remem_seeded_sessionstart"
        );
        assert_eq!(BenchCondition::parse("remem"), None);
        assert_eq!(BenchCondition::parse("remem_preloaded"), None);
        assert_eq!(BenchCondition::parse("curated_file"), None);
        assert_eq!(
            serde_json::to_value(BenchCondition::RememSeededSessionStart)?,
            "remem_seeded_sessionstart"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CodingBenchReport {
    pub schema_version: u32,
    pub generated_at_epoch: i64,
    pub fixture_path: String,
    pub fixture_sha256: String,
    pub remem_rev: String,
    pub source_dirty: Option<bool>,
    pub command: Vec<String>,
    pub artifact_policy: String,
    pub runner: RunnerReport,
    pub runs_per_condition: usize,
    pub ignore_budget: bool,
    pub conditions: Vec<ConditionReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunnerReport {
    pub provider: String,
    pub model: String,
    pub runner: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConditionReport {
    pub name: BenchCondition,
    pub summary: ConditionSummary,
    pub runs: Vec<RunReport>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ConditionSummary {
    pub resolution_rate: f64,
    pub tokens_total_mean: f64,
    pub tokens_total_stddev: f64,
    pub turns_mean: Option<f64>,
    pub wall_time_ms_mean: f64,
    pub wall_time_ms_p95: f64,
    pub failure_counts: BTreeMap<CodingBenchFailureReason, usize>,
    pub memory_failure_counts: BTreeMap<CodingBenchFailureReason, usize>,
    pub human_maintenance_minutes_per_100_sessions: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunReport {
    pub condition: BenchCondition,
    pub task_id: String,
    pub run_index: usize,
    pub resolved: bool,
    pub failure_reason: Option<CodingBenchFailureReason>,
    pub usage: BenchTokenUsage,
    pub turns: Option<usize>,
    pub wall_time_ms: u128,
    pub final_head_sha: Option<String>,
    pub changed_paths: Vec<String>,
    pub unauthorized_path_changes: Vec<String>,
    pub runner_exit_code: Option<i32>,
    pub runner_timed_out: bool,
    pub runtime_contract_failure: bool,
    pub runtime_contract_failure_reason: Option<String>,
    pub context_audit_status: RememContextAuditStatus,
    pub context_audit_failure_reason: Option<String>,
    pub remem_context_audit: Option<RememContextAuditSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curator_log: Option<CuratorLogAttachment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub e2e_pipeline: Option<super::e2e::E2ePipelineTrace>,
    pub score_commands: Vec<CommandReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_contract: Option<CodingMemoryAttribution>,
    #[serde(skip)]
    pub artifacts: RunArtifacts,
    pub workdir: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingBenchFailureReason {
    TestFailure,
    Timeout,
    CompileFailure,
    WrongFileModified,
    IgnoredMemory,
    MissingMemory,
    StaleMemoryFollowed,
    IrrelevantMemoryDistracted,
    OverContextBudget,
    AgentHallucinatedMemory,
    OracleInconclusive,
}

impl CodingBenchFailureReason {
    pub const fn is_memory_specific(self) -> bool {
        matches!(
            self,
            Self::IgnoredMemory
                | Self::MissingMemory
                | Self::StaleMemoryFollowed
                | Self::IrrelevantMemoryDistracted
                | Self::AgentHallucinatedMemory
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct CodingMemoryAttributionInput {
    pub injected_memory_ids: Vec<i64>,
    pub relevant_memory_ids: Vec<i64>,
    pub forbidden_memory_ids: Vec<i64>,
    pub gold_required_facts: Vec<String>,
    pub gold_forbidden_facts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CodingMemoryAttribution {
    pub injected_memory_ids: Vec<i64>,
    pub used_memory_ids: Vec<i64>,
    pub citation_precision: f64,
    pub citation_recall: f64,
    pub stale_used_count: usize,
    pub irrelevant_injection_count: usize,
    pub missing_relevant_memory_count: usize,
    pub memory_helped: bool,
    pub memory_hurt: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct BenchTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandReport {
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    #[serde(skip)]
    pub stdout_artifact: String,
    #[serde(skip)]
    pub stderr_artifact: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunArtifacts {
    pub runner_stdout: String,
    pub runner_stderr: String,
    pub final_diff: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CuratorLogAttachment {
    pub schema_version: u32,
    pub task_id: String,
    pub target_blind: bool,
    pub memory_sha256: String,
    pub curator_log_sha256: String,
    pub final_char_count: usize,
    pub history_session_count: usize,
    pub maintenance_minutes: f64,
    pub update_count: u64,
    pub deletion_count: u64,
    pub conflict_resolution_count: u64,
}

fn default_timeout_ms() -> u64 {
    900_000
}
