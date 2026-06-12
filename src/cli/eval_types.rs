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
