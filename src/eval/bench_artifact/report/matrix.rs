use std::collections::{BTreeMap, BTreeSet};

use super::CodingTaskOutcome;

pub(in crate::eval::bench_artifact) const CLAIM_BEARING_CODING_CONDITIONS: [&str; 3] =
    ["no_memory", "remem_e2e", "curated_file_budgeted"];
pub(in crate::eval::bench_artifact) const CLAIM_BEARING_TASK_IDS: [&str; 16] = [
    "ticket-key-memory-convention",
    "retention-window-memory-policy",
    "slug-normalizer-contract",
    "duration-parser-root-cause",
    "api-version-stale-memory",
    "storage-driver-stale-memory",
    "session-budget-footer",
    "markdown-cell-negative-constraint",
    "workstream-title-continuity",
    "workstream-status-continuity",
    "feature-flag-multi-hop-routing",
    "notification-channel-multi-hop",
    "user-date-format-preference",
    "user-status-line-preference",
    "conflicting-endpoint-current-choice",
    "ambiguous-owner-abstention",
];
pub(in crate::eval::bench_artifact) const REGISTERED_RUN_INDICES: [u32; 3] = [0, 1, 2];
pub(in crate::eval::bench_artifact) const REGISTERED_BENCHMARK_ID: &str = "issue385-v1";
pub(in crate::eval::bench_artifact) const REGISTERED_BENCHMARK_VERSION: &str = "official-v1";
pub(in crate::eval::bench_artifact) const REGISTERED_RUN_PHASE: &str = "official";
pub(in crate::eval::bench_artifact) const REGISTERED_MATRIX_NAMESPACE: &str =
    "issue385-v1/official-v1";

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct CodingMatrixReadiness {
    pub(super) has_registered_identity: bool,
    pub(super) has_required_conditions: bool,
    pub(super) has_identical_task_sets: bool,
    pub(super) has_three_runs_per_task: bool,
    pub(super) ready: bool,
}

pub(in crate::eval::bench_artifact) fn has_claim_bearing_coding_conditions(
    conditions: &BTreeSet<String>,
) -> bool {
    conditions.len() == CLAIM_BEARING_CODING_CONDITIONS.len()
        && CLAIM_BEARING_CODING_CONDITIONS
            .iter()
            .all(|condition| conditions.contains(*condition))
}

pub(in crate::eval::bench_artifact) fn has_claim_ready_coding_matrix(
    artifact_verifier_passed: bool,
    outcomes: &[CodingTaskOutcome],
) -> bool {
    artifact_verifier_passed && coding_matrix_readiness(outcomes).ready
}

pub(super) fn coding_matrix_readiness(outcomes: &[CodingTaskOutcome]) -> CodingMatrixReadiness {
    let mut reports: BTreeMap<&str, Vec<&CodingTaskOutcome>> = BTreeMap::new();
    for outcome in outcomes {
        reports
            .entry(&outcome.report_path)
            .or_default()
            .push(outcome);
    }

    let mut readiness = CodingMatrixReadiness::default();
    for report_outcomes in reports.values() {
        let has_registered_identity = report_outcomes.iter().all(|outcome| {
            outcome.benchmark_id == REGISTERED_BENCHMARK_ID
                && outcome.benchmark_version == REGISTERED_BENCHMARK_VERSION
                && outcome.run_phase == REGISTERED_RUN_PHASE
                && outcome.matrix_namespace == REGISTERED_MATRIX_NAMESPACE
        });
        readiness.has_registered_identity |= has_registered_identity;
        if !has_registered_identity {
            continue;
        }

        let mut conditions: BTreeMap<&str, BTreeMap<&str, BTreeSet<u32>>> = BTreeMap::new();
        for outcome in report_outcomes {
            conditions
                .entry(&outcome.condition)
                .or_default()
                .entry(&outcome.task_id)
                .or_default()
                .insert(outcome.run_index);
        }
        let condition_names = conditions
            .keys()
            .map(|condition| (*condition).to_string())
            .collect();
        let has_required = has_claim_bearing_coding_conditions(&condition_names);
        readiness.has_required_conditions |= has_required;
        if !has_required {
            continue;
        }

        let task_sets = CLAIM_BEARING_CODING_CONDITIONS.map(|condition| {
            conditions[condition]
                .keys()
                .copied()
                .collect::<BTreeSet<_>>()
        });
        let registered_tasks = BTreeSet::from(CLAIM_BEARING_TASK_IDS);
        let has_identical_tasks = task_sets[0] == registered_tasks
            && task_sets[1] == registered_tasks
            && task_sets[2] == registered_tasks;
        readiness.has_identical_task_sets |= has_identical_tasks;
        if !has_identical_tasks {
            continue;
        }

        let registered_indices = BTreeSet::from(REGISTERED_RUN_INDICES);
        let has_three_runs = CLAIM_BEARING_CODING_CONDITIONS.iter().all(|condition| {
            conditions[*condition]
                .values()
                .all(|run_indices| run_indices == &registered_indices)
        });
        readiness.has_three_runs_per_task |= has_three_runs;
        readiness.ready |= has_three_runs;
    }
    readiness
}
