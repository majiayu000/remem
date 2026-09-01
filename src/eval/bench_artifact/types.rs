use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::eval::coding_bench::{RememContextAuditSnapshot, RememContextAuditStatus};
use crate::eval::memory_bench::types::{
    MemoryBenchPolicyOutcome, MemoryBenchPolicySummary, MemoryBenchSuiteFixture,
};

#[derive(Debug, Clone)]
pub struct BenchVerifyOptions {
    pub root: PathBuf,
    pub claim_registry_path: PathBuf,
}

impl BenchVerifyOptions {
    pub fn new(root: impl Into<PathBuf>, claim_registry_path: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            claim_registry_path: claim_registry_path.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct VerifiedArtifact<T> {
    pub path: String,
    pub sha256: String,
    pub value: T,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BenchVerifyFailure {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchVerifyReport {
    pub schema_version: u32,
    pub root: String,
    pub passed: bool,
    pub manifests_checked: usize,
    pub reports_checked: usize,
    pub run_artifacts_checked: usize,
    pub artifact_files_checked: usize,
    pub failures: Vec<BenchVerifyFailure>,
    pub authority_verdict: AuthorityVerdict,
    #[serde(skip)]
    pub(crate) verified_artifacts: VerifiedBenchmarkArtifacts,
}

#[derive(Serialize)]
struct PersistedBenchVerifyReport<'a> {
    schema_version: u32,
    root: &'a str,
    passed: bool,
    manifests_checked: usize,
    reports_checked: usize,
    run_artifacts_checked: usize,
    artifact_files_checked: usize,
    failures: &'a [BenchVerifyFailure],
}

pub(crate) fn serialize_persisted_bench_verify_report<S>(
    report: &BenchVerifyReport,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    PersistedBenchVerifyReport {
        schema_version: report.schema_version,
        root: &report.root,
        passed: report.passed,
        manifests_checked: report.manifests_checked,
        reports_checked: report.reports_checked,
        run_artifacts_checked: report.run_artifacts_checked,
        artifact_files_checked: report.artifact_files_checked,
        failures: &report.failures,
    }
    .serialize(serializer)
}

#[derive(Debug, Clone, Default)]
pub(crate) struct VerifiedBenchmarkArtifacts {
    pub memory_suites: Vec<VerifiedArtifact<MemoryBenchSuiteFixture>>,
    pub manifests: Vec<VerifiedArtifact<PublicBenchmarkManifest>>,
    pub reports: Vec<VerifiedArtifact<PublicBenchmarkReport>>,
    pub memory_runs: Vec<VerifiedArtifact<MemoryRunArtifact>>,
    pub coding_runs: Vec<VerifiedArtifact<CodingRunArtifact>>,
    pub security_policy_outcomes: BTreeMap<String, MemoryBenchPolicyOutcome>,
    pub claim_registry: Option<VerifiedArtifact<ClaimRegistryPolicy>>,
    pub curator_logs: BTreeMap<String, VerifiedArtifact<CuratorLogArtifact>>,
    pub official_coding_tests: BTreeMap<String, VerifiedArtifact<OfficialCodingTestEvidence>>,
    pub treatment_maintenance:
        BTreeMap<String, VerifiedArtifact<OfficialCodingMaintenanceEvidence>>,
    pub official_evidence_authenticated: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum AuthorityStatus {
    #[serde(rename = "PASS")]
    Pass,
    #[serde(rename = "FAIL")]
    Fail,
    #[serde(rename = "INSUFFICIENT")]
    Insufficient,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorityVerdict {
    pub schema_version: u32,
    pub status: AuthorityStatus,
    pub consumed_bytes: BTreeMap<String, String>,
    pub implementation: ImplementationAuthorityBinding,
    pub security: SecurityAuthorityVerdict,
    pub gh931: Gh931AuthorityVerdict,
    pub release: ReleaseAuthorityVerdict,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImplementationAuthorityBinding {
    pub build_git_sha: Option<String>,
    pub checkout_git_sha: Option<String>,
    pub build_source_dirty: Option<bool>,
    pub checkout_source_dirty: Option<bool>,
    pub build_production_input_tree_sha256: Option<String>,
    pub checkout_production_input_tree_sha256: Option<String>,
    pub production_pathspec_sha256: Option<String>,
    pub executable_source_equivalent: bool,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Gh931AuthorityVerdict {
    pub status: AuthorityStatus,
    pub measurement_ready: bool,
    pub registry: Gh931RegistryBinding,
    pub report: Option<Gh931ReportBinding>,
    pub completeness: Gh931Completeness,
    pub condition_completion: Vec<Gh931ConditionCompletion>,
    pub paired_statistics: Vec<super::report::CodingPairedStatistic>,
    pub maintenance: Gh931MaintenanceVerdict,
    pub stop_loss: Gh931StopLossVerdict,
    pub claims: Vec<Gh931ClaimVerdict>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Gh931ConditionCompletion {
    pub condition: String,
    pub eligible_started: usize,
    pub resolved: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Gh931MaintenanceVerdict {
    pub status: AuthorityStatus,
    pub curator_tasks: usize,
    pub curator_sessions: usize,
    pub curator_minutes: Option<f64>,
    pub curated_minutes_per_100_sessions: Option<f64>,
    pub remem_sessions: Option<usize>,
    pub remem_minutes_per_100_sessions: Option<f64>,
    pub reduction_pct: Option<f64>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Gh931RegistryBinding {
    pub path: Option<String>,
    pub sha256: Option<String>,
    pub schema_version: Option<u32>,
    pub issue: Option<String>,
    pub locked: bool,
    pub policy_valid: bool,
    pub declared_statuses: Vec<AuthorityStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Gh931ReportBinding {
    pub path: String,
    pub sha256: String,
    pub conditions: Vec<String>,
    pub models_by_condition: BTreeMap<String, Vec<Value>>,
    pub platforms: Vec<String>,
    pub producing_shas: Vec<String>,
    pub production_input_trees: Vec<String>,
    pub source_dirty_attestations: Vec<Option<bool>>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Gh931Completeness {
    pub expected_tasks: usize,
    pub expected_conditions: usize,
    pub expected_runs_per_task: usize,
    pub expected_runs: usize,
    pub observed_runs: usize,
    pub complete: bool,
    pub attempts_ready: bool,
    pub machine_outcomes_ready: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Gh931StopLossVerdict {
    pub status: AuthorityStatus,
    pub eligible_runs: usize,
    pub memory_hurt_rate_pct: Option<f64>,
    pub stale_memory_followed_rate_pct: Option<f64>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Gh931ClaimVerdict {
    pub id: String,
    pub status: AuthorityStatus,
    pub declared_registry_status: AuthorityStatus,
    pub treatment: String,
    pub control: String,
    pub metric: String,
    pub allowed_wording: Vec<String>,
    pub forbidden_wording: Vec<String>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaimRegistryPolicy {
    pub schema_version: u32,
    pub issue: String,
    pub locked: bool,
    pub claims: Vec<ClaimRegistryClaimPolicy>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaimRegistryClaimPolicy {
    pub id: String,
    pub comparison: ClaimRegistryComparison,
    pub metric: String,
    pub gate: ClaimRegistryGate,
    pub status: AuthorityStatus,
    pub allowed_wording: Vec<String>,
    pub forbidden_wording: Vec<String>,
    pub supporting_report: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaimRegistryComparison {
    pub treatment: String,
    pub control: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum ClaimRegistryGate {
    Superiority(ClaimSuperiorityGate),
    NonInferiority(ClaimNonInferiorityGate),
    StopLoss(ClaimStopLossGate),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaimSuperiorityGate {
    pub min_effect_pp: f64,
    pub ci_lower_bound_pp_gt: f64,
    pub ci_level: f64,
    pub statistical_unit: String,
    pub method: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaimNonInferiorityGate {
    pub non_inferiority_margin_pp: f64,
    pub human_maintenance_reduction_min_pct: f64,
    pub ci_level: f64,
    pub statistical_unit: String,
    pub method: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaimStopLossGate {
    pub memory_hurt_max_pct: f64,
    pub stale_memory_followed_max_pct: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CuratorLogArtifact {
    pub schema_version: u32,
    pub condition: String,
    pub task_id: String,
    pub target_blind: bool,
    pub budget: CuratorBudget,
    pub sessions: Vec<CuratorSession>,
    pub totals: CuratorTotals,
    pub final_char_count: usize,
    pub final_file_sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CuratorBudget {
    pub minutes_per_session: f64,
    pub max_chars: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CuratorSession {
    pub episode_id: String,
    pub minutes_spent: f64,
    pub edit_count: u64,
    pub deletion_count: u64,
    pub conflict_resolution_count: u64,
    pub chars_after: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CuratorTotals {
    pub maintenance_minutes: f64,
    pub update_count: u64,
    pub deletion_count: u64,
    pub conflict_resolution_count: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OfficialCodingTestEvidence {
    pub schema_version: u32,
    pub task_id: String,
    pub condition: String,
    pub run_index: u32,
    pub attempt_id: String,
    pub commands: Vec<OfficialCodingCommandResult>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OfficialCodingCommandResult {
    pub command: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

impl OfficialCodingTestEvidence {
    pub(crate) fn command_validation_error(&self) -> Option<&'static str> {
        if self.commands.is_empty() {
            return Some("official coding test evidence requires commands");
        }
        self.commands
            .iter()
            .any(|result| {
                result.command.trim().is_empty() || result.timed_out != result.exit_code.is_none()
            })
            .then_some("official coding command result is internally inconsistent")
    }

    pub(crate) fn matches_registered_scorer_commands(&self, task_id: &str) -> bool {
        let Ok(fixture): Result<Value, _> = serde_json::from_str(include_str!(
            "../../../eval/coding-bench/fixtures/tasks.json"
        )) else {
            return false;
        };
        let Some(task) = fixture["tasks"].as_array().and_then(|tasks| {
            tasks
                .iter()
                .find(|task| task["id"].as_str() == Some(task_id))
        }) else {
            return false;
        };
        let Some(expected) = task["score"]["commands"].as_array().and_then(|commands| {
            commands
                .iter()
                .map(|command| {
                    command
                        .as_array()?
                        .iter()
                        .map(Value::as_str)
                        .collect::<Option<Vec<_>>>()
                        .map(|parts| parts.join(" "))
                })
                .collect::<Option<Vec<_>>>()
        }) else {
            return false;
        };
        self.commands
            .iter()
            .map(|result| result.command.as_str())
            .eq(expected.iter().map(String::as_str))
    }

    pub(crate) fn resolved(&self) -> bool {
        !self.commands.is_empty()
            && self
                .commands
                .iter()
                .all(|result| !result.timed_out && result.exit_code == Some(0))
    }

    pub(crate) fn failure_reason(&self) -> Option<&'static str> {
        if self.commands.iter().any(|result| result.timed_out) {
            Some("timeout")
        } else if self
            .commands
            .iter()
            .any(|result| result.exit_code != Some(0))
        {
            Some("test_failure")
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OfficialCodingMaintenanceEvidence {
    pub schema_version: u32,
    pub task_id: String,
    pub condition: String,
    pub run_index: u32,
    pub attempt_id: String,
    pub measurement: OfficialCodingMaintenanceMeasurement,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum OfficialCodingMaintenanceMeasurement {
    SupervisorTimed {
        minutes: f64,
        session_count: usize,
    },
    ZeroWork {
        minutes: f64,
        work_events: u64,
        session_count: usize,
    },
}

impl OfficialCodingMaintenanceMeasurement {
    pub(crate) fn is_valid(&self) -> bool {
        match self {
            Self::SupervisorTimed {
                minutes,
                session_count,
            } => minutes.is_finite() && *minutes > 0.0 && *session_count > 0,
            Self::ZeroWork {
                minutes,
                work_events,
                session_count,
            } => minutes.is_finite() && *minutes == 0.0 && *work_events == 0 && *session_count > 0,
        }
    }
}

impl OfficialCodingMaintenanceEvidence {
    pub(crate) fn minutes(&self) -> f64 {
        match &self.measurement {
            OfficialCodingMaintenanceMeasurement::SupervisorTimed { minutes, .. }
            | OfficialCodingMaintenanceMeasurement::ZeroWork { minutes, .. } => *minutes,
        }
    }

    pub(crate) fn session_count(&self) -> usize {
        match &self.measurement {
            OfficialCodingMaintenanceMeasurement::SupervisorTimed { session_count, .. }
            | OfficialCodingMaintenanceMeasurement::ZeroWork { session_count, .. } => {
                *session_count
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SecurityAuthorityVerdict {
    pub status: AuthorityStatus,
    pub runs_recomputed: usize,
    pub policy_failure_count: usize,
    pub reports: Vec<SecurityReportAuthorityVerdict>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecurityReportAuthorityVerdict {
    pub report_path: String,
    pub report_sha256: String,
    pub status: AuthorityStatus,
    pub target: Option<String>,
    pub models: Vec<Value>,
    pub platforms: Vec<String>,
    pub producing_shas: Vec<String>,
    pub production_input_trees: Vec<String>,
    pub source_dirty_attestations: Vec<Option<bool>>,
    pub runs_recomputed: usize,
    pub policy_failure_count: usize,
    pub policy_summary: Option<MemoryBenchPolicySummary>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReleaseAuthorityVerdict {
    pub status: AuthorityStatus,
    pub ready: bool,
    pub required_targets: Vec<String>,
    pub current_targets: Vec<String>,
    pub missing_targets: Vec<String>,
    pub stale_targets: Vec<String>,
    pub diagnostics: Vec<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<String>,
    #[serde(default)]
    pub run_phase: Option<String>,
    #[serde(default)]
    pub matrix_namespace: Option<String>,
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
    #[serde(default)]
    pub source_dirty: Option<bool>,
    #[serde(default)]
    pub production_input_tree_sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryRunArtifact {
    pub schema_version: u32,
    pub benchmark_id: String,
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
    #[serde(default)]
    pub artifact_sha256: BTreeMap<String, String>,
    #[serde(default)]
    pub suite_content_identity: Option<String>,
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
    pub benchmark_id: String,
    pub benchmark_version: String,
    pub run_phase: String,
    pub matrix_namespace: String,
    pub layer: BenchmarkLayer,
    pub condition: String,
    pub task_id: String,
    pub run_index: u32,
    #[serde(default)]
    pub attempt_id: Option<String>,
    #[serde(default)]
    pub target_started: Option<bool>,
    #[serde(default)]
    pub model: Value,
    pub environment: RunEnvironment,
    pub resolved: bool,
    pub failure_reason: Option<String>,
    pub metrics: CodingRunMetrics,
    #[serde(default)]
    pub memory_contract: Option<CodingMemoryContract>,
    #[serde(default)]
    pub context_audit_status: Option<RememContextAuditStatus>,
    #[serde(default)]
    pub context_audit_failure_reason: Option<String>,
    #[serde(default)]
    pub remem_context_audit: Option<RememContextAuditSnapshot>,
    #[serde(default)]
    pub injected_context_sha256: Option<String>,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CodingMemoryContract {
    #[serde(default)]
    pub injected_memory_ids: Vec<i64>,
    #[serde(default)]
    pub used_memory_ids: Vec<i64>,
    pub citation_precision: f64,
    pub citation_recall: f64,
    pub stale_used_count: u64,
    pub irrelevant_injection_count: u64,
    pub missing_relevant_memory_count: u64,
    pub memory_helped: bool,
    pub memory_hurt: bool,
}
