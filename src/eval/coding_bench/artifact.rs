use anyhow::{bail, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

use crate::eval::current_memory_contracts::{
    CurrentMemoryContractEvalReport, CurrentMemoryContractRateMetric,
};

use super::types::{CodingBenchFailureReason, CodingMemoryAttribution};

pub const CODING_AGENT_AB_SPEC_PATH: &str = "docs/specs/issue385-coding-agent-ab/TECH.md";
pub const CURRENT_MEMORY_CONTRACT_SPEC_PATH: &str = "docs/specs/current-memory-contracts/TECH.md";
pub const MIN_RUNS_PER_CONDITION: usize = 3;
const REQUIRED_CONDITIONS: [CodingBenchCondition; 3] = [
    CodingBenchCondition::Remem,
    CodingBenchCondition::NoMemory,
    CodingBenchCondition::CuratedFile,
];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CodingBenchReport {
    pub schema_version: u32,
    pub benchmark_spec_path: &'static str,
    pub current_memory_contract_spec_path: &'static str,
    pub runs_per_condition: usize,
    pub conditions: Vec<CodingBenchConditionReport>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CodingBenchConditionReport {
    pub name: CodingBenchCondition,
    pub runs: Vec<CodingBenchRunReport>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CodingBenchRunReport {
    pub condition: CodingBenchCondition,
    pub task_id: String,
    pub run_index: usize,
    #[serde(rename = "resolved")]
    pub task_success: bool,
    #[serde(rename = "failure_reason")]
    pub task_failure_reason: Option<CodingBenchFailureReason>,
    pub memory_contract_status: CodingBenchMemoryContractStatus,
    pub runtime_contract_failure: bool,
    pub runtime_contract_failure_reason: Option<String>,
    pub score: CodingBenchRunScoreEvidence,
    pub metrics: CodingBenchRunMetrics,
    pub final_head_sha: Option<String>,
    pub patch_artifact_path: Option<String>,
    pub unauthorized_path_changes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_contract: Option<CodingMemoryAttribution>,
    pub remem_contract_snapshot: Option<RememContractSnapshot>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CodingBenchRunScoreEvidence {
    pub commands: Vec<CodingBenchScoreCommandEvidence>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CodingBenchScoreCommandEvidence {
    pub command: Vec<String>,
    pub exit_code: i32,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub output_artifact_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CodingBenchRunMetrics {
    pub tokens_input: Option<u64>,
    pub tokens_output: Option<u64>,
    pub tokens_total: Option<u64>,
    pub token_accounting_unsupported_reason: Option<String>,
    pub turns: Option<u64>,
    pub wall_time_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CodingBenchCondition {
    NoMemory,
    Remem,
    CuratedFile,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodingBenchMemoryContractStatus {
    Passed,
    Failed,
    NotApplicable,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RememContractSnapshot {
    pub schema_version: u32,
    pub source: &'static str,
    pub spec_path: &'static str,
    pub captured_at_epoch: i64,
    pub contract_health: RememContractHealth,
    pub citation_precision: CurrentMemoryContractRateMetric,
    pub staleness_handling: RememStalenessHandlingSnapshot,
    pub temporal_fact_eligibility: RememTemporalFactEligibilitySnapshot,
    pub injected_memory_audit: RememInjectedMemoryAuditSnapshot,
    pub usage_feedback_coverage: RememUsageFeedbackCoverageSnapshot,
    pub current_memory_contracts: CurrentMemoryContractEvalReport,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RememContractHealth {
    pub all_checks_passed: bool,
    pub failing_examples: Vec<String>,
    pub warning_count: usize,
    pub warnings: Vec<RememContractWarning>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RememContractWarning {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RememStalenessHandlingSnapshot {
    pub tracked: CurrentMemoryContractRateMetric,
    pub untracked: CurrentMemoryContractRateMetric,
    pub history_tracked: CurrentMemoryContractRateMetric,
    pub verify_before_trust: CurrentMemoryContractRateMetric,
    pub error: CurrentMemoryContractRateMetric,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RememTemporalFactEligibilitySnapshot {
    pub invalidated_fact_exclusion: CurrentMemoryContractRateMetric,
    pub expired_fact_exclusion: CurrentMemoryContractRateMetric,
    pub as_of_fact_retrieval: CurrentMemoryContractRateMetric,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RememInjectedMemoryAuditSnapshot {
    pub injected: CurrentMemoryContractRateMetric,
    pub dropped: CurrentMemoryContractRateMetric,
    pub abstained: CurrentMemoryContractRateMetric,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RememUsageFeedbackCoverageSnapshot {
    pub citation_event_matched: CurrentMemoryContractRateMetric,
    pub usage_event_linked_to_injection_item: CurrentMemoryContractRateMetric,
}

pub fn build_remem_contract_snapshot(
    contract_report: CurrentMemoryContractEvalReport,
    captured_at_epoch: i64,
) -> RememContractSnapshot {
    let mut warnings = Vec::new();
    let mut failing_examples = contract_report.failing_examples.clone();
    if contract_report.metadata.real_db_touched {
        warnings.push(RememContractWarning {
            code: "current_memory_contract_real_db_touched",
            message: "current-memory-contract eval touched the real runtime database".to_string(),
        });
        failing_examples.push("current-memory-contract eval touched real runtime database".into());
    }
    let all_checks_passed =
        contract_report.metrics.all_checks_passed && failing_examples.is_empty();

    RememContractSnapshot {
        schema_version: 1,
        source: "current_memory_contracts",
        spec_path: CURRENT_MEMORY_CONTRACT_SPEC_PATH,
        captured_at_epoch,
        contract_health: RememContractHealth {
            all_checks_passed,
            failing_examples,
            warning_count: warnings.len(),
            warnings,
        },
        citation_precision: contract_report.metrics.usage.citation_event_matched.clone(),
        staleness_handling: RememStalenessHandlingSnapshot {
            tracked: contract_report.metrics.staleness.tracked.clone(),
            untracked: contract_report.metrics.staleness.untracked.clone(),
            history_tracked: contract_report.metrics.staleness.history_tracked.clone(),
            verify_before_trust: contract_report
                .metrics
                .staleness
                .verify_before_trust
                .clone(),
            error: contract_report.metrics.staleness.error.clone(),
        },
        temporal_fact_eligibility: RememTemporalFactEligibilitySnapshot {
            invalidated_fact_exclusion: contract_report
                .metrics
                .temporal
                .invalidated_fact_exclusion
                .clone(),
            expired_fact_exclusion: contract_report
                .metrics
                .temporal
                .expired_fact_exclusion
                .clone(),
            as_of_fact_retrieval: contract_report
                .metrics
                .temporal
                .as_of_fact_retrieval
                .clone(),
        },
        injected_memory_audit: RememInjectedMemoryAuditSnapshot {
            injected: contract_report.metrics.injection.audit_injected.clone(),
            dropped: contract_report.metrics.injection.audit_dropped.clone(),
            abstained: contract_report.metrics.injection.audit_abstained.clone(),
        },
        usage_feedback_coverage: RememUsageFeedbackCoverageSnapshot {
            citation_event_matched: contract_report.metrics.usage.citation_event_matched.clone(),
            usage_event_linked_to_injection_item: contract_report
                .metrics
                .usage
                .usage_event_linked_to_injection_item
                .clone(),
        },
        current_memory_contracts: contract_report,
    }
}

pub fn validate_contract_snapshots(report: &CodingBenchReport) -> Result<()> {
    validate_condition_matrix(report)?;

    for condition in &report.conditions {
        for run in &condition.runs {
            if run.condition != condition.name {
                bail!(
                    "coding bench run {}#{} condition {:?} is nested under {:?}",
                    run.task_id,
                    run.run_index,
                    run.condition,
                    condition.name
                );
            }
            if run.task_success && run.task_failure_reason.is_some() {
                bail!(
                    "coding bench run {}#{} has task_success=true with stale task_failure_reason",
                    run.task_id,
                    run.run_index
                );
            }
            if !run.task_success && run.task_failure_reason.is_none() {
                bail!(
                    "coding bench run {}#{} failed task without task_failure_reason",
                    run.task_id,
                    run.run_index
                );
            }
            if run.runtime_contract_failure
                && run
                    .runtime_contract_failure_reason
                    .as_deref()
                    .is_none_or(|reason| reason.trim().is_empty())
            {
                bail!(
                    "coding bench run {}#{} has runtime contract failure without reason",
                    run.task_id,
                    run.run_index
                );
            }
            if !run.runtime_contract_failure && run.runtime_contract_failure_reason.is_some() {
                bail!(
                    "coding bench run {}#{} has runtime_contract_failure=false with stale runtime_contract_failure_reason",
                    run.task_id,
                    run.run_index
                );
            }
            validate_score_and_patch_evidence(run)?;
            validate_token_accounting(run)?;
            validate_required_run_metrics(run)?;

            match run.condition {
                CodingBenchCondition::Remem => validate_remem_run_contract(run)?,
                CodingBenchCondition::NoMemory | CodingBenchCondition::CuratedFile => {
                    if run.memory_contract_status != CodingBenchMemoryContractStatus::NotApplicable
                    {
                        bail!(
                            "{:?} run {}#{} must mark memory_contract_status as not_applicable",
                            run.condition,
                            run.task_id,
                            run.run_index
                        );
                    }
                    if run.remem_contract_snapshot.is_some() {
                        bail!(
                            "{:?} run {}#{} must not carry a remem contract snapshot",
                            run.condition,
                            run.task_id,
                            run.run_index
                        );
                    }
                    if run.runtime_contract_failure || run.runtime_contract_failure_reason.is_some()
                    {
                        bail!(
                            "{:?} run {}#{} must not report remem runtime contract failure",
                            run.condition,
                            run.task_id,
                            run.run_index
                        );
                    }
                    if run.memory_contract.is_some() {
                        bail!(
                            "{:?} run {}#{} must not carry memory_contract attribution",
                            run.condition,
                            run.task_id,
                            run.run_index
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_condition_matrix(report: &CodingBenchReport) -> Result<()> {
    if report.runs_per_condition < MIN_RUNS_PER_CONDITION {
        bail!(
            "coding bench report runs_per_condition={} is below required minimum {MIN_RUNS_PER_CONDITION}",
            report.runs_per_condition
        );
    }

    let mut reports_by_condition = BTreeMap::new();
    for condition in &report.conditions {
        if reports_by_condition
            .insert(condition.name, condition)
            .is_some()
        {
            bail!(
                "coding bench report contains duplicate {:?} condition",
                condition.name
            );
        }
    }
    for required in REQUIRED_CONDITIONS {
        if !reports_by_condition.contains_key(&required) {
            bail!(
                "coding bench report missing required {:?} condition",
                required
            );
        }
    }
    if reports_by_condition.len() != REQUIRED_CONDITIONS.len() {
        bail!(
            "coding bench report condition count={} does not match required condition count={}",
            reports_by_condition.len(),
            REQUIRED_CONDITIONS.len()
        );
    }

    let mut task_ids = BTreeSet::new();
    for condition in reports_by_condition.values() {
        for run in &condition.runs {
            task_ids.insert(run.task_id.clone());
        }
    }
    if task_ids.is_empty() {
        bail!("coding bench report has no task runs");
    }

    for required in REQUIRED_CONDITIONS {
        let Some(condition) = reports_by_condition.get(&required) else {
            bail!(
                "coding bench report missing required {:?} condition",
                required
            );
        };
        let mut seen = BTreeSet::new();
        for run in &condition.runs {
            if run.run_index >= report.runs_per_condition {
                bail!(
                    "{:?} run {}#{} is outside runs_per_condition={}",
                    required,
                    run.task_id,
                    run.run_index,
                    report.runs_per_condition
                );
            }
            if !seen.insert((run.task_id.clone(), run.run_index)) {
                bail!(
                    "{:?} condition repeats run {}#{}",
                    required,
                    run.task_id,
                    run.run_index
                );
            }
        }
        for task_id in &task_ids {
            for run_index in 0..report.runs_per_condition {
                if !seen.contains(&(task_id.clone(), run_index)) {
                    bail!(
                        "{:?} condition missing run {}#{}",
                        required,
                        task_id,
                        run_index
                    );
                }
            }
        }
    }

    Ok(())
}

fn validate_score_and_patch_evidence(run: &CodingBenchRunReport) -> Result<()> {
    if run.score.commands.is_empty() {
        bail!(
            "coding bench run {}#{} is missing score command evidence",
            run.task_id,
            run.run_index
        );
    }
    for (command_index, command) in run.score.commands.iter().enumerate() {
        if command.command.is_empty() || command.command.iter().any(|part| part.trim().is_empty()) {
            bail!(
                "coding bench run {}#{} score command {command_index} has blank command argv",
                run.task_id,
                run.run_index
            );
        }
        let has_inline_output = command
            .stdout
            .as_deref()
            .is_some_and(|stdout| !stdout.trim().is_empty())
            || command
                .stderr
                .as_deref()
                .is_some_and(|stderr| !stderr.trim().is_empty());
        let has_output_artifact = command
            .output_artifact_path
            .as_deref()
            .is_some_and(|path| !path.trim().is_empty());
        if !has_inline_output && !has_output_artifact {
            bail!(
                "coding bench run {}#{} score command {command_index} has no output evidence",
                run.task_id,
                run.run_index
            );
        }
    }

    let has_final_head_sha = run
        .final_head_sha
        .as_deref()
        .is_some_and(|sha| !sha.trim().is_empty());
    let has_patch_artifact = run
        .patch_artifact_path
        .as_deref()
        .is_some_and(|path| !path.trim().is_empty());
    if !has_final_head_sha && !has_patch_artifact {
        bail!(
            "coding bench run {}#{} is missing final_head_sha or patch_artifact_path",
            run.task_id,
            run.run_index
        );
    }
    if let Some(sha) = run.final_head_sha.as_deref() {
        let sha = sha.trim();
        if sha.len() != 40 || !sha.chars().all(|ch| ch.is_ascii_hexdigit()) {
            bail!(
                "coding bench run {}#{} final_head_sha is not a full git SHA",
                run.task_id,
                run.run_index
            );
        }
    }
    if run
        .unauthorized_path_changes
        .iter()
        .any(|path| path.trim().is_empty())
    {
        bail!(
            "coding bench run {}#{} has blank unauthorized path change",
            run.task_id,
            run.run_index
        );
    }

    Ok(())
}

fn validate_token_accounting(run: &CodingBenchRunReport) -> Result<()> {
    let token_fields = [
        run.metrics.tokens_input,
        run.metrics.tokens_output,
        run.metrics.tokens_total,
    ];
    let token_fields_present = token_fields.iter().filter(|value| value.is_some()).count();
    let unsupported_reason = run
        .metrics
        .token_accounting_unsupported_reason
        .as_deref()
        .map(str::trim);
    let has_unsupported_reason = unsupported_reason.is_some_and(|reason| !reason.is_empty());

    match (token_fields_present, has_unsupported_reason) {
        (3, false) => {
            let input = run.metrics.tokens_input.unwrap_or_default();
            let output = run.metrics.tokens_output.unwrap_or_default();
            let total = run.metrics.tokens_total.unwrap_or_default();
            if input.saturating_add(output) != total {
                bail!(
                    "coding bench run {}#{} tokens_total={} does not equal tokens_input + tokens_output ({input} + {output})",
                    run.task_id,
                    run.run_index,
                    total
                );
            }
            Ok(())
        }
        (3, true) => bail!(
            "coding bench run {}#{} has token metrics and token_accounting_unsupported_reason",
            run.task_id,
            run.run_index
        ),
        (0, true) => Ok(()),
        (0, false) => bail!(
            "coding bench run {}#{} is missing token accounting without token_accounting_unsupported_reason",
            run.task_id,
            run.run_index
        ),
        (_, _) => bail!(
            "coding bench run {}#{} must record complete token accounting or token_accounting_unsupported_reason",
            run.task_id,
            run.run_index
        ),
    }
}

fn validate_required_run_metrics(run: &CodingBenchRunReport) -> Result<()> {
    if run.metrics.turns.is_none() {
        bail!(
            "coding bench run {}#{} is missing turns",
            run.task_id,
            run.run_index
        );
    }
    if run.metrics.wall_time_ms.is_none() {
        bail!(
            "coding bench run {}#{} is missing wall_time_ms",
            run.task_id,
            run.run_index
        );
    }
    Ok(())
}

fn validate_remem_run_contract(run: &CodingBenchRunReport) -> Result<()> {
    validate_memory_attribution(run)?;
    let snapshot = run.remem_contract_snapshot.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "remem run {}#{} is missing current memory contract snapshot",
            run.task_id,
            run.run_index
        )
    })?;
    let embedded_contract_passed = snapshot.current_memory_contracts.metrics.all_checks_passed
        && snapshot
            .current_memory_contracts
            .failing_examples
            .is_empty()
        && !snapshot.current_memory_contracts.metadata.real_db_touched;
    if snapshot.contract_health.all_checks_passed != embedded_contract_passed {
        bail!(
            "remem run {}#{} contract_health does not match embedded current_memory_contracts",
            run.task_id,
            run.run_index
        );
    }
    let contract_failed = !embedded_contract_passed;
    let expected_status = if contract_failed {
        CodingBenchMemoryContractStatus::Failed
    } else {
        CodingBenchMemoryContractStatus::Passed
    };
    if run.memory_contract_status != expected_status {
        bail!(
            "remem run {}#{} memory_contract_status={:?} does not match contract health={}",
            run.task_id,
            run.run_index,
            run.memory_contract_status,
            snapshot.contract_health.all_checks_passed
        );
    }
    if contract_failed != run.runtime_contract_failure {
        bail!(
            "remem run {}#{} runtime_contract_failure={} does not match contract health={}",
            run.task_id,
            run.run_index,
            run.runtime_contract_failure,
            snapshot.contract_health.all_checks_passed
        );
    }
    Ok(())
}

fn validate_memory_attribution(run: &CodingBenchRunReport) -> Result<()> {
    let attribution = run.memory_contract.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "remem run {}#{} is missing memory_contract attribution",
            run.task_id,
            run.run_index
        )
    })?;
    validate_rate(attribution.citation_precision, "citation_precision", run)?;
    validate_rate(attribution.citation_recall, "citation_recall", run)?;
    ensure_unique_positive_ids(&attribution.injected_memory_ids, "injected_memory_ids", run)?;
    ensure_unique_positive_ids(&attribution.used_memory_ids, "used_memory_ids", run)?;
    if attribution.memory_helped && attribution.memory_hurt {
        bail!(
            "remem run {}#{} memory_contract cannot mark both memory_helped and memory_hurt",
            run.task_id,
            run.run_index
        );
    }
    if run
        .task_failure_reason
        .is_some_and(CodingBenchFailureReason::is_memory_specific)
        && !attribution.memory_hurt
    {
        bail!(
            "remem run {}#{} has memory-specific failure without memory_hurt=true",
            run.task_id,
            run.run_index
        );
    }
    Ok(())
}

fn validate_rate(value: f64, field: &str, run: &CodingBenchRunReport) -> Result<()> {
    if !(0.0..=1.0).contains(&value) || !value.is_finite() {
        bail!(
            "remem run {}#{} memory_contract {field} must be a finite rate between 0 and 1",
            run.task_id,
            run.run_index
        );
    }
    Ok(())
}

fn ensure_unique_positive_ids(ids: &[i64], field: &str, run: &CodingBenchRunReport) -> Result<()> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if *id <= 0 {
            bail!(
                "remem run {}#{} memory_contract {field} contains non-positive id",
                run.task_id,
                run.run_index
            );
        }
        if !seen.insert(*id) {
            bail!(
                "remem run {}#{} memory_contract {field} contains duplicate id",
                run.task_id,
                run.run_index
            );
        }
    }
    Ok(())
}
