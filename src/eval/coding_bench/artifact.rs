use anyhow::{bail, Result};
use serde::Serialize;

use crate::eval::current_memory_contracts::{
    CurrentMemoryContractEvalReport, CurrentMemoryContractRateMetric,
};

pub const CODING_AGENT_AB_SPEC_PATH: &str = "docs/specs/issue385-coding-agent-ab/TECH.md";
pub const CURRENT_MEMORY_CONTRACT_SPEC_PATH: &str = "docs/specs/current-memory-contracts/TECH.md";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CodingBenchReport {
    pub schema_version: u32,
    pub benchmark_spec_path: &'static str,
    pub current_memory_contract_spec_path: &'static str,
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
    pub task_failure_reason: Option<String>,
    pub memory_contract_status: CodingBenchMemoryContractStatus,
    pub runtime_contract_failure: bool,
    pub runtime_contract_failure_reason: Option<String>,
    pub metrics: CodingBenchRunMetrics,
    pub remem_contract_snapshot: Option<RememContractSnapshot>,
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

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
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
            if !run.task_success && run.task_failure_reason.is_none() {
                bail!(
                    "coding bench run {}#{} failed task without task_failure_reason",
                    run.task_id,
                    run.run_index
                );
            }
            if run.runtime_contract_failure && run.runtime_contract_failure_reason.is_none() {
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
            validate_token_accounting(run)?;

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
                }
            }
        }
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
        (3, false) => Ok(()),
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

fn validate_remem_run_contract(run: &CodingBenchRunReport) -> Result<()> {
    let snapshot = run.remem_contract_snapshot.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "remem run {}#{} is missing current memory contract snapshot",
            run.task_id,
            run.run_index
        )
    })?;
    let contract_failed = !snapshot.contract_health.all_checks_passed;
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
