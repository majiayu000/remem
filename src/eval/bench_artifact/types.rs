use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct BenchVerifyOptions {
    pub root: PathBuf,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BenchVerifyFailure {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BenchVerifyReport {
    pub schema_version: u32,
    pub root: String,
    pub passed: bool,
    pub manifests_checked: usize,
    pub reports_checked: usize,
    pub run_artifacts_checked: usize,
    pub artifact_files_checked: usize,
    pub failures: Vec<BenchVerifyFailure>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkLayer {
    MemorySystemCapability,
    CodingAgentOutcome,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PublicBenchmarkManifest {
    pub schema_version: u32,
    pub benchmark_id: String,
    pub layer: BenchmarkLayer,
    pub version: String,
    pub created_at_epoch: i64,
    pub source_policy: SourcePolicy,
    #[serde(default)]
    pub conditions: Vec<String>,
    #[serde(default)]
    pub reports: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SourcePolicy {
    pub private_user_memory_allowed: bool,
    pub requires_temp_remem_data_dir: bool,
    pub external_dataset_revision: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PublicBenchmarkReport {
    pub schema_version: u32,
    pub benchmark_id: String,
    pub benchmark_version: String,
    pub layer: BenchmarkLayer,
    #[serde(default)]
    pub conditions: Vec<String>,
    #[serde(default)]
    pub schema_refs: Vec<String>,
    #[serde(default)]
    pub run_artifacts: Vec<String>,
    #[serde(default)]
    pub aggregate_metrics: Value,
    pub claim_level: String,
    pub verifier: ReportVerifierMetadata,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReportVerifierMetadata {
    pub required: bool,
    pub schema_version: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RunEnvironment {
    pub os: String,
    pub arch: String,
    pub remem_commit: String,
    pub remem_data_dir: String,
    #[serde(default)]
    pub docker_image_digest: Option<String>,
    #[serde(default)]
    pub fixture_revision: Option<String>,
    #[serde(default)]
    pub repo_base_commit: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryRunArtifact {
    pub schema_version: u32,
    pub benchmark_version: String,
    pub layer: BenchmarkLayer,
    pub suite: String,
    pub condition: String,
    pub task_id: String,
    pub run_index: u32,
    pub reference_time_epoch: i64,
    #[serde(default)]
    pub reader_model: Value,
    pub environment: RunEnvironment,
    #[serde(default)]
    pub answer: Value,
    pub retrieval: MemoryRetrievalEvidence,
    pub evidence: MemoryCitationEvidence,
    #[serde(default)]
    pub metrics: Value,
    pub diagnosis: MemoryDiagnosis,
    #[serde(default)]
    pub artifacts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryRetrievalEvidence {
    #[serde(default)]
    pub retrieved_memory_ids: Vec<i64>,
    #[serde(default)]
    pub retrieved_supporting_evidence_ids: Vec<String>,
    #[serde(default)]
    pub gold_supporting_event_ids: Vec<String>,
    #[serde(default)]
    pub missing_supporting_evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryCitationEvidence {
    #[serde(default)]
    pub cited_memory_ids: Vec<i64>,
    #[serde(default)]
    pub cited_event_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryDiagnosis {
    pub write_side_gap: bool,
    pub retrieval_side_gap: bool,
    pub reader_gap: bool,
    pub policy_abstention: bool,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CodingRunArtifact {
    pub schema_version: u32,
    pub benchmark_version: String,
    pub layer: BenchmarkLayer,
    pub condition: String,
    pub task_id: String,
    pub run_index: u32,
    #[serde(default)]
    pub model: Value,
    pub environment: RunEnvironment,
    pub resolved: bool,
    pub failure_reason: Option<String>,
    pub metrics: CodingRunMetrics,
    #[serde(default)]
    pub memory_contract: Value,
    #[serde(default)]
    pub artifacts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CodingRunMetrics {
    pub tokens_input: Option<u64>,
    pub tokens_output: Option<u64>,
    pub tokens_total: Option<u64>,
    pub turns: Option<u64>,
    pub wall_time_ms: Option<u64>,
    pub tool_calls: Option<u64>,
    pub commands_run: Option<u64>,
}
