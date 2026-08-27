//! GH969 executable ship matrix and outcome scorecard.
//!
//! Existing eval and benchmark evidence is composed here without converting
//! missing official runs into zeroes or successful claims.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::eval::bench_artifact::PublicBaselineReport;
use crate::eval::gates::EvalGateDelta;

mod authority;
mod rows;
mod scorecard;

pub const DEFAULT_PUBLIC_ROOT: &str = "eval/public";
pub const DEFAULT_SECURITY_REPORT: &str = "eval/public/memory/reports/adversarial-policy-v2.json";
pub const LINUX_X86_64_SECURITY_REPORT: &str =
    "eval/public/memory/reports/adversarial-policy-v2-linux-x86_64.json";
pub const DEFAULT_CROSS_HOST_CHARTER: &str = "eval/cross-host/benchmark-charter.json";
pub const DEFAULT_CLAIM_REGISTRY: &str = "eval/claims/registry.json";

#[derive(Debug, Clone)]
pub struct ShipMatrixOptions {
    pub baseline_path: String,
    pub thresholds_path: String,
    pub golden_dataset_path: String,
    pub public_root: PathBuf,
    pub security_report_path: PathBuf,
    pub cross_host_charter_path: PathBuf,
    pub claim_registry_path: PathBuf,
}

impl Default for ShipMatrixOptions {
    fn default() -> Self {
        Self {
            baseline_path: crate::eval::gates::DEFAULT_BASELINE_PATH.to_string(),
            thresholds_path: crate::eval::gates::DEFAULT_THRESHOLDS_PATH.to_string(),
            golden_dataset_path: crate::eval::gates::DEFAULT_GOLDEN_DATASET_PATH.to_string(),
            public_root: PathBuf::from(DEFAULT_PUBLIC_ROOT),
            security_report_path: default_security_report_path(),
            cross_host_charter_path: PathBuf::from(DEFAULT_CROSS_HOST_CHARTER),
            claim_registry_path: PathBuf::from(DEFAULT_CLAIM_REGISTRY),
        }
    }
}

fn default_security_report_path() -> PathBuf {
    security_report_for_platform(std::env::consts::OS, std::env::consts::ARCH)
}

fn security_report_for_platform(os: &str, arch: &str) -> PathBuf {
    match (os, arch) {
        ("macos", "aarch64") => PathBuf::from(DEFAULT_SECURITY_REPORT),
        ("linux", "x86_64") => PathBuf::from(LINUX_X86_64_SECURITY_REPORT),
        (os, arch) => PathBuf::from(format!(
            "eval/public/memory/reports/adversarial-policy-v2-{os}-{arch}.json"
        )),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ShipEvidence {
    pub ship_matrix: ShipMatrixReport,
    pub outcome_scorecard: OutcomeScorecard,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShipMatrixReport {
    pub schema_version: u32,
    pub implementation: ImplementationIdentity,
    pub summary: ShipMatrixSummary,
    pub gates: Vec<ShipGateRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImplementationIdentity {
    pub git_sha: Option<String>,
    pub source_dirty: Option<bool>,
    pub package_version: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShipMatrixSummary {
    pub command_passed: bool,
    pub merge_ready: bool,
    pub release_ready: bool,
    pub implementation_identified: bool,
    pub source_clean: bool,
    pub default_on_ready: bool,
    pub cross_host_claim_ready: bool,
    pub coding_outcome_claim_ready: bool,
    pub public_claim_ready: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShipGateRow {
    pub id: &'static str,
    pub owner: &'static str,
    pub status: ShipGateStatus,
    pub blocks: Vec<&'static str>,
    pub required_for_command_success: bool,
    pub claim_level: String,
    pub condition_completeness: String,
    pub config_identity: String,
    pub model_identity: String,
    pub metric_deltas: BTreeMap<String, f64>,
    pub stop_loss_verdict: String,
    pub exclusions: Vec<String>,
    pub evidence: Vec<ArtifactEvidence>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShipGateStatus {
    Pass,
    Fail,
    Incomplete,
    Unavailable,
    #[allow(dead_code)]
    NotApplicable,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactEvidence {
    pub path: String,
    pub state: ArtifactState,
    pub sha256: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactState {
    Verified,
    Present,
    Missing,
    Invalid,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutcomeScorecard {
    pub schema_version: u32,
    pub measurement_states: [MeasurementState; 3],
    pub fields: Vec<ScorecardField>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScorecardField {
    pub id: &'static str,
    pub measurement_state: MeasurementState,
    pub eligible_population: String,
    pub numerator: ScorecardComponent,
    pub denominator: ScorecardComponent,
    pub values: BTreeMap<String, f64>,
    pub threshold: String,
    pub source: Option<String>,
    pub claim_level: String,
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementState {
    Measured,
    Unavailable,
    NotApplicable,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScorecardComponent {
    pub definition: String,
    pub value: Option<f64>,
}

pub(super) struct PublicEvidence {
    pub(super) report: Option<PublicBaselineReport>,
    pub(super) report_error: Option<String>,
    pub(super) security: Option<Value>,
    pub(super) security_error: Option<String>,
    security_authority: authority::SecurityAuthority,
    claim_authority: authority::ClaimAuthority,
}

pub fn build_ship_evidence(
    deltas: &[EvalGateDelta],
    capacity_applicable: bool,
    legacy_gates_passed: bool,
    options: ShipMatrixOptions,
) -> ShipEvidence {
    let implementation = implementation_identity();
    let public = load_public_evidence(&options, &implementation);
    let gates = rows::build_gate_rows(
        deltas,
        capacity_applicable,
        &options,
        &public,
        &implementation,
    );
    let command_passed = legacy_gates_passed
        && gates
            .iter()
            .filter(|gate| gate.required_for_command_success)
            .all(pass_or_not_applicable);
    let implementation_identified = implementation.git_sha.is_some();
    let source_clean = implementation.source_dirty == Some(false);
    let summary = ShipMatrixSummary {
        command_passed,
        merge_ready: merge_is_ready(&gates, legacy_gates_passed),
        release_ready: release_is_ready(
            &gates,
            legacy_gates_passed,
            implementation_identified,
            source_clean,
        ),
        implementation_identified,
        source_clean,
        default_on_ready: required_rows_pass(&gates, "retrieval_default")
            && required_rows_pass(&gates, "context_default"),
        cross_host_claim_ready: gate_passes(&gates, "cross_host"),
        coding_outcome_claim_ready: gate_passes(&gates, "coding_outcome"),
        public_claim_ready: gate_passes(&gates, "public_claim"),
    };
    ShipEvidence {
        ship_matrix: ShipMatrixReport {
            schema_version: 1,
            implementation,
            summary,
            gates,
        },
        outcome_scorecard: scorecard::build_scorecard(&public, &options.security_report_path),
    }
}

fn load_public_evidence(
    options: &ShipMatrixOptions,
    implementation: &ImplementationIdentity,
) -> PublicEvidence {
    let (report, report_error) =
        match crate::eval::bench_artifact::generate_public_baseline_report(&options.public_root) {
            Ok(report) => (Some(report), None),
            Err(error) => (
                None,
                Some(format!("public artifact verification failed: {error:#}")),
            ),
        };
    let (security, security_error) = match read_json_value(&options.security_report_path) {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    };
    let security_authority = authority::verify_security_authority(
        options,
        report.as_ref(),
        security.as_ref(),
        implementation,
    );
    let claim_authority =
        authority::verify_claim_authority(&options.claim_registry_path, implementation);
    PublicEvidence {
        report,
        report_error,
        security,
        security_error,
        security_authority,
        claim_authority,
    }
}

pub(super) fn read_json_value(path: &Path) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("parse {}: {error}", path.display()))
}

pub(super) fn evidence_for_path(path: &Path, desired_state: ArtifactState) -> ArtifactEvidence {
    match fs::read(path) {
        Ok(bytes) => ArtifactEvidence {
            path: path.to_string_lossy().to_string(),
            state: desired_state,
            sha256: Some(format!("{:x}", Sha256::digest(bytes))),
            detail: "exact file content hash".to_string(),
        },
        Err(error) => ArtifactEvidence {
            path: path.to_string_lossy().to_string(),
            state: ArtifactState::Missing,
            sha256: None,
            detail: error.to_string(),
        },
    }
}

pub(super) fn public_model_identity(public: &PublicEvidence) -> String {
    public
        .report
        .as_ref()
        .map(|report| {
            if report.reproducibility.models.is_empty() {
                "artifact_declared_no_model_identity".to_string()
            } else {
                report.reproducibility.models.join(",")
            }
        })
        .unwrap_or_else(|| "unavailable".to_string())
}

fn implementation_identity() -> ImplementationIdentity {
    let git_sha = crate::git_util::git_output_soft(
        Path::new("."),
        &["rev-parse", "--verify", "HEAD^{commit}"],
    )
    .filter(|output| output.status.success())
    .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
    .filter(|value| !value.is_empty());
    let source_dirty = crate::git_util::git_output_soft(Path::new("."), &["status", "--porcelain"])
        .filter(|output| output.status.success())
        .map(|output| !output.stdout.is_empty());
    ImplementationIdentity {
        git_sha,
        source_dirty,
        package_version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    }
}

fn required_rows_pass(gates: &[ShipGateRow], scope: &str) -> bool {
    gates
        .iter()
        .filter(|gate| gate.blocks.contains(&scope))
        .all(pass_or_not_applicable)
}

fn merge_is_ready(gates: &[ShipGateRow], legacy_gates_passed: bool) -> bool {
    legacy_gates_passed && required_rows_pass(gates, "merge")
}

fn release_is_ready(
    gates: &[ShipGateRow],
    legacy_gates_passed: bool,
    implementation_identified: bool,
    source_clean: bool,
) -> bool {
    legacy_gates_passed
        && implementation_identified
        && source_clean
        && required_rows_pass(gates, "release")
}

fn gate_passes(gates: &[ShipGateRow], scope: &str) -> bool {
    gates
        .iter()
        .filter(|gate| gate.blocks.contains(&scope))
        .all(|gate| gate.status == ShipGateStatus::Pass)
}

fn pass_or_not_applicable(gate: &ShipGateRow) -> bool {
    matches!(
        gate.status,
        ShipGateStatus::Pass | ShipGateStatus::NotApplicable
    )
}

#[cfg(test)]
mod tests;
