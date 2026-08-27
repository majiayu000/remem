use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;

use super::types::{
    BenchVerifyOptions, BenchVerifyReport, BenchmarkLayer, PublicBenchmarkReport, RunEnvironment,
    VerifiedArtifact, VerifiedBenchmarkArtifacts,
};

pub(super) mod matrix;
mod statistics;

pub(in crate::eval::bench_artifact) use statistics::coding_paired_statistics;
use statistics::coding_variance;
pub use statistics::{CodingConditionVariance, CodingPairedStatistic};

#[derive(Debug, Clone)]
pub struct BenchReportOptions {
    pub root: PathBuf,
    pub json_out: PathBuf,
    pub markdown_out: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicBaselineReport {
    pub schema_version: u32,
    pub report_id: String,
    pub report_kind: String,
    pub root: String,
    pub created_at_epoch: i64,
    pub claim_level: String,
    pub artifact_verifier: BenchVerifyReport,
    pub summary: BaselineSummary,
    pub reports: Vec<BaselineReportEntry>,
    pub memory_task_outcomes: Vec<MemoryTaskOutcome>,
    pub coding_task_outcomes: Vec<CodingTaskOutcome>,
    pub coding_condition_variance: Vec<CodingConditionVariance>,
    pub coding_paired_statistics: Vec<CodingPairedStatistic>,
    pub failure_decomposition: FailureDecomposition,
    pub reproducibility: ReproducibilitySummary,
    pub claim_gate: ClaimGateSummary,
    pub reproduction_commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaselineSummary {
    pub memory_system: BaselineLayerSummary,
    pub coding_agent: BaselineLayerSummary,
    pub manifest_count: usize,
    pub report_count: usize,
    pub run_artifact_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaselineLayerSummary {
    pub status: String,
    pub report_count: usize,
    pub run_artifact_count: usize,
    pub benchmark_ids: Vec<String>,
    pub conditions: Vec<String>,
    pub claim_levels: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaselineReportEntry {
    pub path: String,
    pub benchmark_id: String,
    pub benchmark_version: String,
    pub layer: BenchmarkLayer,
    pub conditions: Vec<String>,
    pub run_artifact_count: usize,
    pub claim_level: String,
    pub aggregate_metrics: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryTaskOutcome {
    pub report_path: String,
    pub suite: String,
    pub condition: String,
    pub task_id: String,
    pub run_index: u32,
    pub answer_score: Option<f64>,
    pub support_coverage: Option<f64>,
    pub citation_recall: Option<f64>,
    pub write_side_gap: bool,
    pub retrieval_side_gap: bool,
    pub reader_gap: bool,
    pub policy_abstention: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodingTaskOutcome {
    pub report_path: String,
    pub benchmark_id: String,
    pub benchmark_version: String,
    pub run_phase: String,
    pub matrix_namespace: String,
    pub condition: String,
    pub task_id: String,
    pub run_index: u32,
    pub attempt_id: Option<String>,
    pub target_started: Option<bool>,
    pub resolved: bool,
    pub failure_reason: Option<String>,
    pub tokens_total: Option<u64>,
    pub turns: Option<u64>,
    pub wall_time_ms: Option<u64>,
    pub memory_helped: Option<bool>,
    pub memory_hurt: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct FailureDecomposition {
    pub coding_failure_counts: BTreeMap<String, usize>,
    pub coding_memory_failure_counts: BTreeMap<String, usize>,
    pub memory_gap_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReproducibilitySummary {
    pub remem_commits: Vec<String>,
    pub fixture_revisions: Vec<String>,
    pub docker_image_digests: Vec<String>,
    pub repo_base_commits: Vec<String>,
    pub prompt_hashes: Vec<String>,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaimGateSummary {
    pub artifact_verifier_passed: bool,
    pub coding_claim_level: String,
    pub coding_outcome_stop_loss_status: String,
    pub public_sota_status: String,
    pub notes: Vec<String>,
}

#[derive(Default)]
struct BuildState {
    manifest_count: usize,
    report_paths: BTreeSet<PathBuf>,
    reports: Vec<BaselineReportEntry>,
    memory_outcomes: Vec<MemoryTaskOutcome>,
    coding_outcomes: Vec<CodingTaskOutcome>,
    memory_benchmarks: BTreeSet<String>,
    coding_benchmarks: BTreeSet<String>,
    memory_conditions: BTreeSet<String>,
    coding_conditions: BTreeSet<String>,
    memory_claim_levels: BTreeSet<String>,
    coding_claim_levels: BTreeSet<String>,
    failure_decomposition: FailureDecomposition,
    remem_commits: BTreeSet<String>,
    fixture_revisions: BTreeSet<String>,
    docker_image_digests: BTreeSet<String>,
    repo_base_commits: BTreeSet<String>,
    prompt_hashes: BTreeSet<String>,
    models: BTreeSet<String>,
    max_created_at_epoch: i64,
}

pub fn write_public_baseline_report(options: BenchReportOptions) -> Result<PublicBaselineReport> {
    let report = generate_public_baseline_report(&options.root)?;
    write_text_file(&options.json_out, &serde_json::to_string_pretty(&report)?)?;
    write_text_file(
        &options.markdown_out,
        &render_public_baseline_markdown(&report),
    )?;
    Ok(report)
}

pub fn generate_public_baseline_report(root: &Path) -> Result<PublicBaselineReport> {
    let artifact_verifier = super::verify::verify_benchmark_artifacts(BenchVerifyOptions {
        root: root.to_path_buf(),
    })?;
    generate_public_baseline_report_from_verified(root, artifact_verifier)
}

pub(super) fn generate_public_baseline_report_from_verified(
    root: &Path,
    artifact_verifier: BenchVerifyReport,
) -> Result<PublicBaselineReport> {
    let verified = &artifact_verifier.verified_artifacts;

    let mut state = BuildState {
        max_created_at_epoch: 0,
        ..BuildState::default()
    };

    state.manifest_count = verified.manifests.len();
    state.max_created_at_epoch = verified
        .manifests
        .iter()
        .map(|manifest| manifest.value.created_at_epoch)
        .max()
        .unwrap_or_default();
    for report in &verified.reports {
        let logical_path = PathBuf::from(&report.path);
        if state.report_paths.insert(logical_path) {
            load_verified_report(report, verified, &mut state)?;
        }
    }

    let coding_condition_variance = coding_variance(&state.coding_outcomes);
    let coding_paired_statistics =
        coding_paired_statistics(&state.coding_outcomes, artifact_verifier.passed);
    let claim_gate = claim_gate(&artifact_verifier, &state);
    let memory_summary = layer_summary(
        "directional_memory_system_evidence",
        &state.memory_benchmarks,
        &state.memory_conditions,
        &state.memory_claim_levels,
        state.memory_outcomes.len(),
        &[
            "Memory-system capability results are separate from coding-agent outcomes.".to_string(),
            "Committed memory suites are directional until public claim gates pass.".to_string(),
        ],
    );
    let coding_summary = layer_summary(
        "smoke_coding_outcome_evidence",
        &state.coding_benchmarks,
        &state.coding_conditions,
        &state.coding_claim_levels,
        state.coding_outcomes.len(),
        &[
            "Current committed coding artifacts are smoke-only.".to_string(),
            "The #931 claim gate requires no_memory, remem_e2e, and curated_file_budgeted across the registered 16-task fixture with exactly run indices 0, 1, and 2 per task and condition.".to_string(),
        ],
    );

    Ok(PublicBaselineReport {
        schema_version: 1,
        report_id: "public-baseline-directional-v1".to_string(),
        report_kind: "baseline_directional_public_benchmark".to_string(),
        root: root.to_string_lossy().to_string(),
        created_at_epoch: state.max_created_at_epoch,
        claim_level: "directional_only_no_public_claim".to_string(),
        summary: BaselineSummary {
            memory_system: memory_summary,
            coding_agent: coding_summary,
            manifest_count: state.manifest_count,
            report_count: state.reports.len(),
            run_artifact_count: state.memory_outcomes.len() + state.coding_outcomes.len(),
        },
        reports: state.reports,
        memory_task_outcomes: state.memory_outcomes,
        coding_task_outcomes: state.coding_outcomes,
        coding_condition_variance,
        coding_paired_statistics,
        failure_decomposition: state.failure_decomposition,
        reproducibility: ReproducibilitySummary {
            remem_commits: sorted_vec(state.remem_commits),
            fixture_revisions: sorted_vec(state.fixture_revisions),
            docker_image_digests: sorted_vec(state.docker_image_digests),
            repo_base_commits: sorted_vec(state.repo_base_commits),
            prompt_hashes: sorted_vec(state.prompt_hashes),
            models: sorted_vec(state.models),
        },
        claim_gate,
        reproduction_commands: reproduction_commands(),
        artifact_verifier,
    })
}

pub fn render_public_baseline_markdown(report: &PublicBaselineReport) -> String {
    let mut out = String::new();
    out.push_str("# remem Public Baseline Directional Report\n\n");
    out.push_str("Claim level: `");
    out.push_str(&report.claim_level);
    out.push_str("`.\n\n");
    out.push_str("This report separates memory-system capability evidence from coding-agent outcome evidence. It is directional only and does not support SOTA, broad superiority, or coding-task superiority claims.\n\n");

    out.push_str("## Artifact Verification\n\n");
    out.push_str(&format!(
        "- Passed: `{}`\n- Manifests checked: `{}`\n- Reports checked: `{}`\n- Run artifacts checked: `{}`\n- Artifact files checked: `{}`\n\n",
        report.artifact_verifier.passed,
        report.artifact_verifier.manifests_checked,
        report.artifact_verifier.reports_checked,
        report.artifact_verifier.run_artifacts_checked,
        report.artifact_verifier.artifact_files_checked
    ));

    out.push_str("## Memory-System Capability\n\n");
    out.push_str("| Report | Runs | Claim level | Answer score | Support coverage | Citation recall | Non-retention leak rate |\n");
    out.push_str("|---|---:|---|---:|---:|---:|---:|\n");
    for entry in report
        .reports
        .iter()
        .filter(|entry| entry.layer == BenchmarkLayer::MemorySystemCapability)
    {
        out.push_str(&format!(
            "| `{}` | {} | `{}` | {} | {} | {} | {} |\n",
            escape_md(&entry.benchmark_id),
            entry.run_artifact_count,
            escape_md(&entry.claim_level),
            fmt_metric(metric_path(
                &entry.aggregate_metrics,
                &["overall", "answer_score"]
            )),
            fmt_metric(metric_path(
                &entry.aggregate_metrics,
                &["overall", "support_coverage"]
            )),
            fmt_metric(metric_path(
                &entry.aggregate_metrics,
                &["overall", "citation_recall"]
            )),
            fmt_metric(metric_path(
                &entry.aggregate_metrics,
                &["policy", "non_retention_leak_rate"]
            ))
        ));
    }
    out.push('\n');

    out.push_str("## Coding-Agent Outcome\n\n");
    out.push_str("| Condition | Runs | Resolved rate | Token mean | Token variance | Wall-time mean ms | Variance status |\n");
    out.push_str("|---|---:|---:|---:|---:|---:|---|\n");
    for variance in &report.coding_condition_variance {
        out.push_str(&format!(
            "| `{}` | {} | {:.3} | {} | {} | {} | `{}` |\n",
            escape_md(&variance.condition),
            variance.runs,
            variance.resolved_rate,
            fmt_metric(variance.tokens_total_mean),
            fmt_metric(variance.tokens_total_sample_variance),
            fmt_metric(variance.wall_time_ms_mean),
            escape_md(&variance.variance_status)
        ));
    }
    out.push('\n');

    out.push_str("## Paired Coding Statistics\n\n");
    out.push_str("| Comparison | Report | Status | Treatment rate | Control rate | Effect pp | 95% CI pp | Method | Reason |\n");
    out.push_str("|---|---|---|---:|---:|---:|---:|---|---|\n");
    for stat in &report.coding_paired_statistics {
        out.push_str(&format!(
            "| `{}` | {} | `{}` | {} | {} | {} | {} | `{}` | {} |\n",
            escape_md(&stat.comparison_id),
            stat.report_path
                .as_deref()
                .map(|path| format!("`{}`", escape_md(path)))
                .unwrap_or_else(|| "`n/a`".to_string()),
            escape_md(&stat.status),
            fmt_metric(stat.treatment_resolved_rate),
            fmt_metric(stat.control_resolved_rate),
            fmt_metric(stat.effect_pp),
            fmt_ci(stat.ci_lower_pp, stat.ci_upper_pp),
            escape_md(&stat.algorithm),
            stat.insufficient_reason
                .as_deref()
                .map(escape_md)
                .unwrap_or_else(|| "none".to_string())
        ));
    }
    out.push('\n');

    out.push_str("## Coding Task Outcomes\n\n");
    out.push_str("| Task | Condition | Run | Resolved | Failure reason | Tokens | Wall time ms | Memory helped | Memory hurt |\n");
    out.push_str("|---|---|---:|---|---|---:|---:|---|---|\n");
    for run in &report.coding_task_outcomes {
        out.push_str(&format!(
            "| `{}` | `{}` | {} | `{}` | {} | {} | {} | {} | {} |\n",
            escape_md(&run.task_id),
            escape_md(&run.condition),
            run.run_index,
            run.resolved,
            run.failure_reason
                .as_deref()
                .map(|value| format!("`{}`", escape_md(value)))
                .unwrap_or_else(|| "`none`".to_string()),
            fmt_u64(run.tokens_total),
            fmt_u64(run.wall_time_ms),
            fmt_bool(run.memory_helped),
            fmt_bool(run.memory_hurt)
        ));
    }
    out.push('\n');

    out.push_str("## Failure Decomposition\n\n");
    out.push_str("Coding failure counts:\n\n");
    append_count_map(
        &mut out,
        &report.failure_decomposition.coding_failure_counts,
    );
    out.push_str("\nCoding memory-specific failure counts:\n\n");
    append_count_map(
        &mut out,
        &report.failure_decomposition.coding_memory_failure_counts,
    );
    out.push_str("\nMemory gap counts:\n\n");
    append_count_map(&mut out, &report.failure_decomposition.memory_gap_counts);
    out.push('\n');

    out.push_str("## Reproducibility\n\n");
    out.push_str("Run these commands from a clean checkout:\n\n");
    out.push_str("```bash\n");
    for command in &report.reproduction_commands {
        out.push_str(command);
        out.push('\n');
    }
    out.push_str("```\n\n");
    out.push_str("Locks and evidence are recorded in the JSON report under `reproducibility`, including remem commits, fixture revisions, Docker image digests, prompt hashes, model labels, and repo base commits when present.\n\n");

    out.push_str("## Claim Gate\n\n");
    out.push_str(&format!(
        "- Artifact verifier passed: `{}`\n- Coding outcome stop-loss status: `{}`\n- Public SOTA status: `{}`\n",
        report.claim_gate.artifact_verifier_passed,
        report.claim_gate.coding_outcome_stop_loss_status,
        report.claim_gate.public_sota_status
    ));
    for note in &report.claim_gate.notes {
        out.push_str("- ");
        out.push_str(note);
        out.push('\n');
    }

    out
}

fn load_verified_report(
    artifact: &VerifiedArtifact<PublicBenchmarkReport>,
    verified: &VerifiedBenchmarkArtifacts,
    state: &mut BuildState,
) -> Result<()> {
    let report = &artifact.value;
    let report_path = artifact.path.clone();
    match report.layer {
        BenchmarkLayer::MemorySystemCapability => {
            state.memory_benchmarks.insert(report.benchmark_id.clone());
            state.memory_claim_levels.insert(report.claim_level.clone());
            for condition in &report.conditions {
                state.memory_conditions.insert(condition.clone());
            }
            load_memory_runs(&report_path, report, verified, state)?;
        }
        BenchmarkLayer::CodingAgentOutcome => {
            state.coding_benchmarks.insert(report.benchmark_id.clone());
            state.coding_claim_levels.insert(report.claim_level.clone());
            for condition in &report.conditions {
                state.coding_conditions.insert(condition.clone());
            }
            load_coding_runs(&report_path, report, verified, state)?;
        }
    }
    state.reports.push(BaselineReportEntry {
        path: report_path,
        benchmark_id: report.benchmark_id.clone(),
        benchmark_version: report.benchmark_version.clone(),
        layer: report.layer,
        conditions: report.conditions.clone(),
        run_artifact_count: report.run_artifacts.len(),
        claim_level: report.claim_level.clone(),
        aggregate_metrics: report.aggregate_metrics.clone(),
    });
    Ok(())
}

fn load_memory_runs(
    report_path: &str,
    report: &PublicBenchmarkReport,
    verified: &VerifiedBenchmarkArtifacts,
    state: &mut BuildState,
) -> Result<()> {
    for run_path in &report.run_artifacts {
        let run = verified
            .memory_runs
            .iter()
            .find(|artifact| artifact.path == *run_path)
            .with_context(|| format!("verified memory run is missing: {run_path}"))?
            .value
            .clone();
        observe_environment(&run.environment, state);
        observe_model(&run.reader_model, state);
        observe_prompt_hash(&run.reader_model, state);
        if run.diagnosis.write_side_gap {
            increment(
                &mut state.failure_decomposition.memory_gap_counts,
                "write_side_gap",
            );
        }
        if run.diagnosis.retrieval_side_gap {
            increment(
                &mut state.failure_decomposition.memory_gap_counts,
                "retrieval_side_gap",
            );
        }
        if run.diagnosis.reader_gap {
            increment(
                &mut state.failure_decomposition.memory_gap_counts,
                "reader_gap",
            );
        }
        if run.diagnosis.policy_abstention {
            increment(
                &mut state.failure_decomposition.memory_gap_counts,
                "policy_abstention",
            );
        }
        state.memory_outcomes.push(MemoryTaskOutcome {
            report_path: report_path.to_string(),
            suite: run.suite,
            condition: run.condition,
            task_id: run.task_id,
            run_index: run.run_index,
            answer_score: metric_path(&run.metrics, &["answer_score"]),
            support_coverage: metric_path(&run.metrics, &["support_coverage"]),
            citation_recall: metric_path(&run.metrics, &["citation_recall"]),
            write_side_gap: run.diagnosis.write_side_gap,
            retrieval_side_gap: run.diagnosis.retrieval_side_gap,
            reader_gap: run.diagnosis.reader_gap,
            policy_abstention: run.diagnosis.policy_abstention,
        });
    }
    Ok(())
}

fn load_coding_runs(
    report_path: &str,
    report: &PublicBenchmarkReport,
    verified: &VerifiedBenchmarkArtifacts,
    state: &mut BuildState,
) -> Result<()> {
    for run_path in &report.run_artifacts {
        let run = verified
            .coding_runs
            .iter()
            .find(|artifact| artifact.path == *run_path)
            .with_context(|| format!("verified coding run is missing: {run_path}"))?
            .value
            .clone();
        observe_environment(&run.environment, state);
        observe_model(&run.model, state);
        observe_prompt_hash(&run.model, state);
        if let Some(reason) = &run.failure_reason {
            increment(
                &mut state.failure_decomposition.coding_failure_counts,
                reason,
            );
            if is_memory_specific_failure(reason) {
                increment(
                    &mut state.failure_decomposition.coding_memory_failure_counts,
                    reason,
                );
            }
        }
        state.coding_outcomes.push(CodingTaskOutcome {
            report_path: report_path.to_string(),
            benchmark_id: run.benchmark_id,
            benchmark_version: run.benchmark_version,
            run_phase: run.run_phase,
            matrix_namespace: run.matrix_namespace,
            condition: run.condition,
            task_id: run.task_id,
            run_index: run.run_index,
            attempt_id: run.attempt_id,
            target_started: run.target_started,
            resolved: run.resolved,
            failure_reason: run.failure_reason,
            tokens_total: run.metrics.tokens_total,
            turns: run.metrics.turns,
            wall_time_ms: run.metrics.wall_time_ms,
            memory_helped: run
                .memory_contract
                .as_ref()
                .map(|contract| contract.memory_helped),
            memory_hurt: run
                .memory_contract
                .as_ref()
                .map(|contract| contract.memory_hurt),
        });
    }
    Ok(())
}

fn claim_gate(artifact_verifier: &BenchVerifyReport, state: &BuildState) -> ClaimGateSummary {
    let matrix = matrix::coding_matrix_readiness(&state.coding_outcomes);
    let coding_outcome_stop_loss_status = if matrix::has_claim_ready_coding_matrix(
        artifact_verifier.passed,
        &state.coding_outcomes,
    ) {
        "ready_for_stop_loss_evaluation"
    } else {
        "not_evaluated_insufficient_coding_matrix"
    };
    let mut notes = vec![
        "This baseline is directional only and must not be used for coding-task superiority claims.".to_string(),
        "README and release wording must not claim SOTA or coding outcome improvement from this report.".to_string(),
    ];
    if !artifact_verifier.passed {
        notes.push("Coding artifacts must pass the public artifact verifier.".to_string());
    }
    if !matrix.has_registered_identity {
        notes.push(
            "Coding artifacts are not the registered issue385-v1/official-v1 official matrix."
                .to_string(),
        );
    } else if !matrix.has_required_conditions {
        notes.push(
            "Coding artifacts do not yet include no_memory, remem_e2e, and curated_file_budgeted conditions."
                .to_string(),
        );
    } else if !matrix.has_identical_task_sets {
        notes.push(
            "Claim-bearing coding conditions must use the exact registered 16-task set in one report."
                .to_string(),
        );
    } else if !matrix.has_three_runs_per_task {
        notes.push(
            "Coding artifacts do not yet have exactly the registered run indices 0, 1, and 2 per task and condition."
                .to_string(),
        );
    } else if !matrix.has_aggregate_ready_attempts {
        notes.push(
            "Every official tuple must carry a unique verified attempt_id and target_started=true; pre-target failures make the matrix insufficient."
                .to_string(),
        );
    }
    ClaimGateSummary {
        artifact_verifier_passed: artifact_verifier.passed,
        coding_claim_level: "directional_only_no_public_claim".to_string(),
        coding_outcome_stop_loss_status: coding_outcome_stop_loss_status.to_string(),
        public_sota_status: "not_evaluated_no_public_sota_claim".to_string(),
        notes,
    }
}

fn layer_summary(
    status: &str,
    benchmark_ids: &BTreeSet<String>,
    conditions: &BTreeSet<String>,
    claim_levels: &BTreeSet<String>,
    run_artifact_count: usize,
    notes: &[String],
) -> BaselineLayerSummary {
    BaselineLayerSummary {
        status: status.to_string(),
        report_count: claim_levels.len().max(benchmark_ids.len()),
        run_artifact_count,
        benchmark_ids: sorted_vec(benchmark_ids.clone()),
        conditions: sorted_vec(conditions.clone()),
        claim_levels: sorted_vec(claim_levels.clone()),
        notes: notes.to_vec(),
    }
}

fn write_text_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create report directory {}", parent.display()))?;
        }
    }
    fs::write(path, content).with_context(|| format!("write {}", path.display()))
}

fn observe_environment(environment: &RunEnvironment, state: &mut BuildState) {
    insert_non_empty(&mut state.remem_commits, &environment.remem_commit);
    if let Some(value) = &environment.fixture_revision {
        insert_non_empty(&mut state.fixture_revisions, value);
    }
    if let Some(value) = &environment.docker_image_digest {
        insert_non_empty(&mut state.docker_image_digests, value);
    }
    if let Some(value) = &environment.repo_base_commit {
        insert_non_empty(&mut state.repo_base_commits, value);
    }
}

fn observe_model(model: &Value, state: &mut BuildState) {
    let provider = value_string(model, "provider")
        .or_else(|| value_string(model, "agent"))
        .unwrap_or_else(|| "unknown".to_string());
    let name = value_string(model, "model").unwrap_or_else(|| "unknown".to_string());
    insert_non_empty(&mut state.models, &format!("{provider}/{name}"));
}

fn observe_prompt_hash(model: &Value, state: &mut BuildState) {
    if let Some(value) = value_string(model, "prompt_hash") {
        insert_non_empty(&mut state.prompt_hashes, &value);
    }
}

fn value_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn insert_non_empty(set: &mut BTreeSet<String>, value: &str) {
    if !value.trim().is_empty() {
        set.insert(value.to_string());
    }
}

fn metric_path(value: &Value, path: &[&str]) -> Option<f64> {
    let mut cursor = value;
    for segment in path {
        cursor = cursor.get(*segment)?;
    }
    cursor.as_f64()
}

fn increment(map: &mut BTreeMap<String, usize>, key: &str) {
    *map.entry(key.to_string()).or_default() += 1;
}

fn is_memory_specific_failure(reason: &str) -> bool {
    matches!(
        reason,
        "ignored_memory"
            | "missing_memory"
            | "stale_memory_followed"
            | "irrelevant_memory_distracted"
            | "agent_hallucinated_memory"
    )
}

fn sorted_vec(set: BTreeSet<String>) -> Vec<String> {
    set.into_iter().collect()
}

fn reproduction_commands() -> Vec<String> {
    vec![
        "cargo run -- bench verify --root eval/public --json-out /tmp/remem-public-bench-verify.json".to_string(),
        "cargo run -- bench report --root eval/public --json-out eval/public/reports/baseline.json --markdown-out eval/public/reports/baseline.md".to_string(),
        "cargo run -- bench coding --suite issue385-v1 --dry-run --json-out /tmp/remem-issue385-v1-dry-run.json".to_string(),
        "cargo run -- bench memory --suite remem-code-memory --condition remem_default --root eval/public --artifact-prefix memory/artifacts/remem-code-memory-v1 --json-out eval/public/memory/reports/remem-code-memory-v1.json".to_string(),
        "cargo run -- bench memory --suite adversarial-policy --condition remem_default --root eval/public --artifact-prefix memory/artifacts/adversarial-policy-v1 --json-out eval/public/memory/reports/adversarial-policy-v1.json".to_string(),
    ]
}

fn escape_md(value: &str) -> String {
    value.replace('|', "\\|")
}

fn fmt_metric(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| "n/a".to_string())
}

fn fmt_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_string())
}

fn fmt_bool(value: Option<bool>) -> String {
    value
        .map(|value| format!("`{value}`"))
        .unwrap_or_else(|| "`n/a`".to_string())
}

fn fmt_ci(lower: Option<f64>, upper: Option<f64>) -> String {
    match (lower, upper) {
        (Some(lower), Some(upper)) => format!("{lower:.3} to {upper:.3}"),
        _ => "n/a".to_string(),
    }
}

fn append_count_map(out: &mut String, map: &BTreeMap<String, usize>) {
    if map.is_empty() {
        out.push_str("- none\n");
        return;
    }
    for (key, count) in map {
        out.push_str(&format!("- `{}`: {}\n", escape_md(key), count));
    }
}
