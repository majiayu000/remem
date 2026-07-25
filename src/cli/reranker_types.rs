use clap::Subcommand;

#[derive(Subcommand)]
pub(in crate::cli) enum RerankerAction {
    /// Download and verify a local reranker model into the remem data directory.
    Download {
        /// Model preset: bge-reranker-base.
        #[arg(long)]
        model: Option<String>,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
    /// Show rerank configuration and local model inventory readiness.
    Status {
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
}
