use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::{matrix, CodingTaskOutcome};

const CODING_PAIRED_BOOTSTRAP_REPLICATES: usize = 10_000;
const CODING_PAIRED_BOOTSTRAP_SEED: u64 = 931;
const CODING_PAIRED_BOOTSTRAP_CI_LEVEL: f64 = 0.95;
const CODING_PAIRED_BOOTSTRAP_METHOD: &str = "task-cluster paired bootstrap";
const CODING_PAIRED_BOOTSTRAP_ALGORITHM: &str = "task_cluster_paired_bootstrap_v1";
const CODING_PAIRED_BOOTSTRAP_PERCENTILE_RULE: &str =
    "sorted floor(alpha/2 * n), ceil((1 - alpha/2) * n) - 1";

const CODING_PAIRED_COMPARISONS: [(&str, &str, &str); 2] = [
    ("remem-e2e-vs-no-memory-v1", "remem_e2e", "no_memory"),
    (
        "remem-e2e-vs-curated-file-budgeted-v1",
        "remem_e2e",
        "curated_file_budgeted",
    ),
];

#[derive(Debug, Clone, Serialize)]
pub struct CodingConditionVariance {
    pub condition: String,
    pub runs: usize,
    pub resolved_rate: f64,
    pub tokens_total_mean: Option<f64>,
    pub tokens_total_sample_variance: Option<f64>,
    pub wall_time_ms_mean: Option<f64>,
    pub wall_time_ms_sample_variance: Option<f64>,
    pub variance_status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodingPairedStatistic {
    pub comparison_id: String,
    pub treatment: String,
    pub control: String,
    pub metric: String,
    pub report_path: Option<String>,
    pub status: String,
    pub insufficient_reason: Option<String>,
    pub tasks: usize,
    pub runs_per_task: usize,
    pub treatment_resolved_rate: Option<f64>,
    pub control_resolved_rate: Option<f64>,
    pub effect_pp: Option<f64>,
    pub ci_level: f64,
    pub ci_lower_pp: Option<f64>,
    pub ci_upper_pp: Option<f64>,
    pub bootstrap_replicates: usize,
    pub bootstrap_seed: u64,
    pub statistical_unit: String,
    pub method: String,
    pub algorithm: String,
    pub percentile_rule: String,
}

pub(super) fn coding_variance(outcomes: &[CodingTaskOutcome]) -> Vec<CodingConditionVariance> {
    let mut grouped: BTreeMap<String, Vec<&CodingTaskOutcome>> = BTreeMap::new();
    for outcome in outcomes {
        grouped
            .entry(outcome.condition.clone())
            .or_default()
            .push(outcome);
    }
    grouped
        .into_iter()
        .map(|(condition, runs)| {
            let resolved = runs.iter().filter(|run| run.resolved).count();
            let tokens = runs
                .iter()
                .filter_map(|run| run.tokens_total.map(|value| value as f64))
                .collect::<Vec<_>>();
            let wall = runs
                .iter()
                .filter_map(|run| run.wall_time_ms.map(|value| value as f64))
                .collect::<Vec<_>>();
            let variance_status = if runs.len() >= 3 {
                "satisfied"
            } else {
                "insufficient_runs_for_variance"
            }
            .to_string();
            CodingConditionVariance {
                condition,
                runs: runs.len(),
                resolved_rate: resolved as f64 / runs.len() as f64,
                tokens_total_mean: mean(&tokens),
                tokens_total_sample_variance: sample_variance(&tokens),
                wall_time_ms_mean: mean(&wall),
                wall_time_ms_sample_variance: sample_variance(&wall),
                variance_status,
            }
        })
        .collect()
}

pub(in crate::eval::bench_artifact) fn coding_paired_statistics(
    outcomes: &[CodingTaskOutcome],
    artifact_verifier_passed: bool,
) -> Vec<CodingPairedStatistic> {
    let mut by_report: BTreeMap<&str, Vec<&CodingTaskOutcome>> = BTreeMap::new();
    for outcome in outcomes {
        by_report
            .entry(&outcome.report_path)
            .or_default()
            .push(outcome);
    }

    let complete_reports = by_report
        .into_iter()
        .filter(|(_, report_outcomes)| report_outcomes_structurally_complete(report_outcomes))
        .collect::<Vec<_>>();

    if complete_reports.is_empty() {
        return CODING_PAIRED_COMPARISONS
            .into_iter()
            .map(|(comparison_id, treatment, control)| {
                insufficient_coding_paired_statistic(
                    comparison_id,
                    treatment,
                    control,
                    None,
                    0,
                    0,
                    "requires one verified issue385-v1/official-v1 report containing no_memory, remem_e2e, and curated_file_budgeted for all 16 registered tasks with run indices 0, 1, and 2",
                )
            })
            .collect();
    }

    let mut statistics = Vec::new();
    for (report_path, report_outcomes) in complete_reports {
        for (comparison_id, treatment, control) in CODING_PAIRED_COMPARISONS {
            if !artifact_verifier_passed {
                statistics.push(insufficient_coding_paired_statistic(
                    comparison_id,
                    treatment,
                    control,
                    Some(report_path),
                    matrix::CLAIM_BEARING_TASK_IDS.len(),
                    matrix::REGISTERED_RUN_INDICES.len(),
                    "the benchmark artifact verifier did not pass; integrity-invalid tuples cannot be aggregated",
                ));
            } else if !matrix::report_attempts_ready_for_aggregation(&report_outcomes) {
                statistics.push(insufficient_coding_paired_statistic(
                    comparison_id,
                    treatment,
                    control,
                    Some(report_path),
                    matrix::CLAIM_BEARING_TASK_IDS.len(),
                    matrix::REGISTERED_RUN_INDICES.len(),
                    "one or more tuples lack a unique verified attempt_id or target_started=true; pre-target failures cannot be scored as zero",
                ));
            } else {
                statistics.push(compute_coding_paired_statistic(
                    report_path,
                    &report_outcomes,
                    comparison_id,
                    treatment,
                    control,
                ));
            }
        }
    }
    statistics
}

fn insufficient_coding_paired_statistic(
    comparison_id: &str,
    treatment: &str,
    control: &str,
    report_path: Option<&str>,
    tasks: usize,
    runs_per_task: usize,
    reason: &str,
) -> CodingPairedStatistic {
    CodingPairedStatistic {
        comparison_id: comparison_id.to_string(),
        treatment: treatment.to_string(),
        control: control.to_string(),
        metric: "resolved_rate".to_string(),
        report_path: report_path.map(ToString::to_string),
        status: "insufficient".to_string(),
        insufficient_reason: Some(reason.to_string()),
        tasks,
        runs_per_task,
        treatment_resolved_rate: None,
        control_resolved_rate: None,
        effect_pp: None,
        ci_level: CODING_PAIRED_BOOTSTRAP_CI_LEVEL,
        ci_lower_pp: None,
        ci_upper_pp: None,
        bootstrap_replicates: CODING_PAIRED_BOOTSTRAP_REPLICATES,
        bootstrap_seed: CODING_PAIRED_BOOTSTRAP_SEED,
        statistical_unit: "task".to_string(),
        method: CODING_PAIRED_BOOTSTRAP_METHOD.to_string(),
        algorithm: CODING_PAIRED_BOOTSTRAP_ALGORITHM.to_string(),
        percentile_rule: CODING_PAIRED_BOOTSTRAP_PERCENTILE_RULE.to_string(),
    }
}

fn compute_coding_paired_statistic(
    report_path: &str,
    outcomes: &[&CodingTaskOutcome],
    comparison_id: &str,
    treatment: &str,
    control: &str,
) -> CodingPairedStatistic {
    let treatment_means = resolved_means_by_task(outcomes, treatment);
    let control_means = resolved_means_by_task(outcomes, control);
    let mut treatment_rates = Vec::new();
    let mut control_rates = Vec::new();
    let mut paired_effects = Vec::new();

    for task_id in matrix::CLAIM_BEARING_TASK_IDS {
        let treatment_rate = treatment_means[task_id];
        let control_rate = control_means[task_id];
        treatment_rates.push(treatment_rate);
        control_rates.push(control_rate);
        paired_effects.push(treatment_rate - control_rate);
    }

    let effect_pp = mean_required(&paired_effects) * 100.0;
    let (ci_lower_pp, ci_upper_pp) = bootstrap_paired_ci(&paired_effects);

    CodingPairedStatistic {
        comparison_id: comparison_id.to_string(),
        treatment: treatment.to_string(),
        control: control.to_string(),
        metric: "resolved_rate".to_string(),
        report_path: Some(report_path.to_string()),
        status: "computed".to_string(),
        insufficient_reason: None,
        tasks: matrix::CLAIM_BEARING_TASK_IDS.len(),
        runs_per_task: matrix::REGISTERED_RUN_INDICES.len(),
        treatment_resolved_rate: Some(mean_required(&treatment_rates)),
        control_resolved_rate: Some(mean_required(&control_rates)),
        effect_pp: Some(effect_pp),
        ci_level: CODING_PAIRED_BOOTSTRAP_CI_LEVEL,
        ci_lower_pp: Some(ci_lower_pp),
        ci_upper_pp: Some(ci_upper_pp),
        bootstrap_replicates: CODING_PAIRED_BOOTSTRAP_REPLICATES,
        bootstrap_seed: CODING_PAIRED_BOOTSTRAP_SEED,
        statistical_unit: "task".to_string(),
        method: CODING_PAIRED_BOOTSTRAP_METHOD.to_string(),
        algorithm: CODING_PAIRED_BOOTSTRAP_ALGORITHM.to_string(),
        percentile_rule: CODING_PAIRED_BOOTSTRAP_PERCENTILE_RULE.to_string(),
    }
}

fn report_outcomes_structurally_complete(outcomes: &[&CodingTaskOutcome]) -> bool {
    if outcomes.len()
        != matrix::CLAIM_BEARING_CODING_CONDITIONS.len()
            * matrix::CLAIM_BEARING_TASK_IDS.len()
            * matrix::REGISTERED_RUN_INDICES.len()
    {
        return false;
    }
    if !outcomes.iter().all(|outcome| {
        outcome.benchmark_id == matrix::REGISTERED_BENCHMARK_ID
            && outcome.benchmark_version == matrix::REGISTERED_BENCHMARK_VERSION
            && outcome.run_phase == matrix::REGISTERED_RUN_PHASE
            && outcome.matrix_namespace == matrix::REGISTERED_MATRIX_NAMESPACE
    }) {
        return false;
    }

    let mut conditions: BTreeMap<&str, BTreeMap<&str, BTreeSet<u32>>> = BTreeMap::new();
    for outcome in outcomes {
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
    if !matrix::has_claim_bearing_coding_conditions(&condition_names) {
        return false;
    }

    let registered_tasks = BTreeSet::from(matrix::CLAIM_BEARING_TASK_IDS);
    let registered_indices = BTreeSet::from(matrix::REGISTERED_RUN_INDICES);
    matrix::CLAIM_BEARING_CODING_CONDITIONS
        .iter()
        .all(|condition| {
            let task_runs = &conditions[*condition];
            task_runs.keys().copied().collect::<BTreeSet<_>>() == registered_tasks
                && task_runs
                    .values()
                    .all(|run_indices| run_indices == &registered_indices)
        })
}

fn resolved_means_by_task<'a>(
    outcomes: &'a [&'a CodingTaskOutcome],
    condition: &str,
) -> BTreeMap<&'a str, f64> {
    let mut grouped: BTreeMap<&str, Vec<bool>> = BTreeMap::new();
    for outcome in outcomes
        .iter()
        .copied()
        .filter(|outcome| outcome.condition == condition)
    {
        grouped
            .entry(&outcome.task_id)
            .or_default()
            .push(outcome.resolved);
    }
    grouped
        .into_iter()
        .map(|(task_id, runs)| {
            let resolved = runs.iter().filter(|resolved| **resolved).count();
            (task_id, resolved as f64 / runs.len() as f64)
        })
        .collect()
}

fn bootstrap_paired_ci(task_effects: &[f64]) -> (f64, f64) {
    let mut rng = SplitMix64::new(CODING_PAIRED_BOOTSTRAP_SEED);
    let task_count = task_effects.len();
    let mut samples = Vec::with_capacity(CODING_PAIRED_BOOTSTRAP_REPLICATES);
    for _ in 0..CODING_PAIRED_BOOTSTRAP_REPLICATES {
        let mut sum = 0.0;
        for _ in 0..task_count {
            sum += task_effects[rng.next_index(task_count)];
        }
        samples.push(sum / task_count as f64 * 100.0);
    }
    samples.sort_by(f64::total_cmp);

    let alpha = 1.0 - CODING_PAIRED_BOOTSTRAP_CI_LEVEL;
    let lower_index = ((alpha / 2.0) * samples.len() as f64).floor() as usize;
    let upper_index = ((1.0 - alpha / 2.0) * samples.len() as f64).ceil() as usize;
    let upper_index = upper_index.saturating_sub(1).min(samples.len() - 1);
    (samples[lower_index], samples[upper_index])
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| mean_required(values))
}

fn mean_required(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn sample_variance(values: &[f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    let average = mean(values)?;
    Some(
        values
            .iter()
            .map(|value| {
                let delta = value - average;
                delta * delta
            })
            .sum::<f64>()
            / (values.len() - 1) as f64,
    )
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn next_index(&mut self, len: usize) -> usize {
        (self.next_u64() % len as u64) as usize
    }
}
