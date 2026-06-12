use clap::Args;

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
