use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct CodingBenchOptions {
    pub fixture_path: String,
    pub runs_per_condition: usize,
    pub json_out: String,
    pub condition: Option<String>,
    pub task: Option<String>,
    pub keep_workdirs: bool,
    pub dry_run: bool,
    pub runner: String,
    pub codex_bin: String,
    pub model: String,
    pub provider: Option<String>,
    pub reasoning_effort: String,
    pub ignore_budget: bool,
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
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CodingBenchTask {
    pub id: String,
    pub prompt: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    pub score: ScoreSpec,
    #[serde(default)]
    pub memories: Vec<SeedMemory>,
    #[serde(default)]
    pub curated_context: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScoreSpec {
    #[serde(default)]
    pub commands: Vec<Vec<String>>,
    #[serde(default)]
    pub hidden_files: BTreeMap<String, String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchCondition {
    NoMemory,
    Remem,
    CuratedFile,
}

impl BenchCondition {
    pub const ALL: [Self; 3] = [Self::NoMemory, Self::Remem, Self::CuratedFile];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoMemory => "no_memory",
            Self::Remem => "remem",
            Self::CuratedFile => "curated_file",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "no_memory" => Some(Self::NoMemory),
            "remem" => Some(Self::Remem),
            "curated_file" => Some(Self::CuratedFile),
            _ => None,
        }
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
}

#[derive(Debug, Clone, Serialize)]
pub struct RunReport {
    pub condition: BenchCondition,
    pub task_id: String,
    pub run_index: usize,
    pub resolved: bool,
    pub failure_reason: Option<String>,
    pub usage: BenchTokenUsage,
    pub turns: Option<usize>,
    pub wall_time_ms: u128,
    pub final_head_sha: Option<String>,
    pub changed_paths: Vec<String>,
    pub unauthorized_path_changes: Vec<String>,
    pub runner_exit_code: Option<i32>,
    pub runner_timed_out: bool,
    pub score_commands: Vec<CommandReport>,
    #[serde(skip)]
    pub artifacts: RunArtifacts,
    pub workdir: Option<String>,
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

fn default_timeout_ms() -> u64 {
    900_000
}
