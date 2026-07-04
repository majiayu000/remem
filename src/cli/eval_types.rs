use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli) enum BenchAction {
    /// Verify public benchmark artifact schemas, layout, logs, and isolation evidence.
    Verify(BenchVerifyArgs),
    /// Run a deterministic public memory capability suite.
    Memory(BenchMemoryArgs),
    /// Run or dry-run the public coding-agent benchmark suite.
    Coding(BenchCodingArgs),
    /// Generate the directional public benchmark baseline report from committed artifacts.
    Report(BenchReportArgs),
}

#[derive(Args)]
pub(in crate::cli) struct BenchVerifyArgs {
    /// Public benchmark artifact root.
    #[arg(long, default_value = "eval/public")]
    pub(in crate::cli) root: String,
    /// Verification report output path.
    #[arg(long)]
    pub(in crate::cli) json_out: String,
}

#[derive(Args)]
pub(in crate::cli) struct BenchMemoryArgs {
    /// Memory benchmark suite id.
    #[arg(long, default_value = crate::eval::memory_bench::types::DEFAULT_SUITE)]
    pub(in crate::cli) suite: String,
    /// Restrict to one condition: no_memory, oracle_evidence, complete_stored_memory, retrieved_memory, or remem_default.
    #[arg(long)]
    pub(in crate::cli) condition: Option<String>,
    /// Public benchmark artifact root.
    #[arg(long, default_value = crate::eval::memory_bench::types::DEFAULT_PUBLIC_ROOT)]
    pub(in crate::cli) root: String,
    /// Artifact directory prefix under --root when --json-out is inside --root.
    #[arg(long)]
    pub(in crate::cli) artifact_prefix: Option<String>,
    /// Memory benchmark report output path.
    #[arg(long)]
    pub(in crate::cli) json_out: String,
}

#[derive(Args)]
pub(in crate::cli) struct BenchCodingArgs {
    /// Coding benchmark suite id. Currently supports issue385-v1.
    #[arg(long, default_value = "issue385-v1")]
    pub(in crate::cli) suite: String,
    /// Override the fixture path for local debugging.
    #[arg(long)]
    pub(in crate::cli) fixture: Option<String>,
    /// Repetitions for each selected condition/task pair.
    #[arg(long, default_value = "3")]
    pub(in crate::cli) runs_per_condition: usize,
    /// JSON report output path.
    #[arg(long)]
    pub(in crate::cli) json_out: String,
    /// Restrict to one condition: no_memory, remem, or curated_file.
    #[arg(long)]
    pub(in crate::cli) condition: Option<String>,
    /// Restrict to one task id.
    #[arg(long)]
    pub(in crate::cli) task: Option<String>,
    /// Select the full v1 task pack or smoke subset.
    #[arg(long, default_value = "full")]
    pub(in crate::cli) task_set: String,
    /// Preserve temporary workdirs for inspection.
    #[arg(long)]
    pub(in crate::cli) keep_workdirs: bool,
    /// Validate fixtures and print the planned matrix without invoking an agent.
    #[arg(long)]
    pub(in crate::cli) dry_run: bool,
    /// Agent runner implementation.
    #[arg(long, default_value = "codex")]
    pub(in crate::cli) runner: String,
    /// Codex executable path when --runner=codex.
    #[arg(long, default_value = "codex")]
    pub(in crate::cli) codex_bin: String,
    /// Model passed to the coding-agent runner.
    #[arg(long, default_value = "gpt-5.5")]
    pub(in crate::cli) model: String,
    /// Optional provider label recorded in reports.
    #[arg(long)]
    pub(in crate::cli) provider: Option<String>,
    /// Codex model_reasoning_effort config override.
    #[arg(long, default_value = "medium")]
    pub(in crate::cli) reasoning_effort: String,
    /// Record that the caller intentionally ignored budget gates for this manual run.
    #[arg(long)]
    pub(in crate::cli) ignore_budget: bool,
}

#[derive(Args)]
pub(in crate::cli) struct BenchReportArgs {
    /// Public benchmark artifact root.
    #[arg(long, default_value = "eval/public")]
    pub(in crate::cli) root: String,
    /// Baseline report JSON output path.
    #[arg(long)]
    pub(in crate::cli) json_out: String,
    /// Baseline report Markdown output path.
    #[arg(long)]
    pub(in crate::cli) markdown_out: String,
}

#[derive(Args)]
pub(in crate::cli) struct EvalExtractionArgs {
    #[arg(long, default_value = crate::eval::extraction::DEFAULT_CORPUS_PATH)]
    pub(in crate::cli) corpus: String,
    #[arg(long, default_value = crate::eval::extraction::DEFAULT_BASELINE_PATH)]
    pub(in crate::cli) baseline: String,
    #[arg(long)]
    pub(in crate::cli) json: bool,
    #[arg(long)]
    pub(in crate::cli) check_baseline: bool,
}

#[derive(Args)]
pub(in crate::cli) struct EvalGatesArgs {
    #[arg(long, default_value = crate::eval::gates::DEFAULT_BASELINE_PATH)]
    pub(in crate::cli) baseline: String,
    #[arg(long, default_value = crate::eval::gates::DEFAULT_THRESHOLDS_PATH)]
    pub(in crate::cli) thresholds: String,
    #[arg(long, default_value = crate::eval::gates::DEFAULT_GOLDEN_DATASET_PATH)]
    pub(in crate::cli) golden_dataset: String,
    #[arg(long)]
    pub(in crate::cli) json_out: Option<String>,
    #[arg(long)]
    pub(in crate::cli) json: bool,
    #[arg(long, hide = true)]
    pub(in crate::cli) simulate_golden_regression: bool,
    #[arg(long, hide = true)]
    pub(in crate::cli) simulate_capacity_regression: bool,
}

#[derive(Args)]
pub(in crate::cli) struct EvalProviderComparisonArgs {
    #[arg(long, default_value = crate::eval::provider_comparison::DEFAULT_DATASET_PATH)]
    pub(in crate::cli) dataset: String,
    #[arg(long, short = 'k', default_value = "5")]
    pub(in crate::cli) k: usize,
    #[arg(long, default_value = crate::eval::provider_comparison::DEFAULT_REPORT_PATH)]
    pub(in crate::cli) json_out: String,
    #[arg(long)]
    pub(in crate::cli) json: bool,
    /// Permit remote API embedding calls for the api comparison row.
    #[arg(long)]
    pub(in crate::cli) allow_api: bool,
}

#[derive(Args)]
pub(in crate::cli) struct EvalGraphDecisionArgs {
    #[arg(long, default_value = crate::eval::graph_decision::DEFAULT_DATASET_PATH)]
    pub(in crate::cli) dataset: String,
    #[arg(long, short = 'k', default_value = "5")]
    pub(in crate::cli) k: usize,
    #[arg(long, default_value = crate::eval::graph_decision::DEFAULT_REPORT_PATH)]
    pub(in crate::cli) json_out: String,
    #[arg(long)]
    pub(in crate::cli) json: bool,
}

#[derive(Args)]
pub(in crate::cli) struct EvalAssociativeBaselineArgs {
    #[arg(long, default_value = crate::eval::associative::DEFAULT_DATASET_PATH)]
    pub(in crate::cli) dataset: String,
    #[arg(long, short = 'k', default_value = "5")]
    pub(in crate::cli) k: usize,
    #[arg(long, default_value = crate::eval::associative::DEFAULT_REPORT_PATH)]
    pub(in crate::cli) json_out: String,
    #[arg(long)]
    pub(in crate::cli) json: bool,
}

#[derive(Args)]
pub(in crate::cli) struct EvalCapacityArgs {
    #[arg(long, default_value = crate::eval::capacity::DEFAULT_DATASET_PATH)]
    pub(in crate::cli) dataset: String,
    #[arg(long, default_value_t = 42)]
    pub(in crate::cli) seed: u64,
    #[arg(long, default_value = "1,10")]
    pub(in crate::cli) scales: String,
    #[arg(long, short = 'k', default_value = "5")]
    pub(in crate::cli) k: usize,
    #[arg(long)]
    pub(in crate::cli) json_out: Option<String>,
    #[arg(long)]
    pub(in crate::cli) json: bool,
}

#[derive(Args)]
pub(in crate::cli) struct EvalWeightGridArgs {
    #[arg(long, default_value = crate::eval::weight_grid::DEFAULT_DATASET_PATH)]
    pub(in crate::cli) dataset: String,
    #[arg(long, short = 'k', default_value = "5")]
    pub(in crate::cli) k: usize,
    #[arg(long, default_value = crate::eval::weight_grid::DEFAULT_REPORT_PATH)]
    pub(in crate::cli) json_out: String,
    #[arg(long)]
    pub(in crate::cli) json: bool,
}

#[derive(Args)]
pub(in crate::cli) struct EvalCodingBenchArgs {
    /// Coding-agent benchmark fixture.
    #[arg(long)]
    pub(in crate::cli) fixture: String,
    /// Repetitions for each selected condition/task pair.
    #[arg(long)]
    pub(in crate::cli) runs_per_condition: usize,
    /// JSON report output path. Required unless --dry-run is set.
    #[arg(long)]
    pub(in crate::cli) json_out: Option<String>,
    /// Restrict to one condition: no_memory, remem, or curated_file.
    #[arg(long)]
    pub(in crate::cli) condition: Option<String>,
    /// Restrict to one task id.
    #[arg(long)]
    pub(in crate::cli) task: Option<String>,
    /// Select the full v1 task pack or smoke subset.
    #[arg(long, default_value = "full")]
    pub(in crate::cli) task_set: String,
    /// Preserve temporary workdirs for inspection.
    #[arg(long)]
    pub(in crate::cli) keep_workdirs: bool,
    /// Validate fixtures and print the planned matrix without invoking an agent.
    #[arg(long)]
    pub(in crate::cli) dry_run: bool,
    /// Agent runner implementation.
    #[arg(long, default_value = "codex")]
    pub(in crate::cli) runner: String,
    /// Codex executable path when --runner=codex.
    #[arg(long, default_value = "codex")]
    pub(in crate::cli) codex_bin: String,
    /// Model passed to the coding-agent runner.
    #[arg(long, default_value = "gpt-5.5")]
    pub(in crate::cli) model: String,
    /// Optional provider label recorded in reports.
    #[arg(long)]
    pub(in crate::cli) provider: Option<String>,
    /// Codex model_reasoning_effort config override.
    #[arg(long, default_value = "medium")]
    pub(in crate::cli) reasoning_effort: String,
    /// Record that the caller intentionally ignored budget gates for this manual run.
    #[arg(long)]
    pub(in crate::cli) ignore_budget: bool,
}
