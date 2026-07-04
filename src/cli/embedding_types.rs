use clap::Subcommand;

#[derive(Subcommand)]
pub(in crate::cli) enum EmbeddingAction {
    /// Download a local semantic embedding model into the remem data directory.
    Download {
        /// Model preset: multilingual-e5-small or bge-m3.
        #[arg(long)]
        model: Option<String>,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
    /// Show local semantic model inventory and active-provider readiness.
    Status {
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
    /// Backfill the active embedding profile for searchable memories.
    Backfill {
        /// Rows per measured write batch.
        #[arg(long, default_value_t = 1000)]
        batch: i64,
        /// Optional maximum rows to process before stopping.
        #[arg(long)]
        limit: Option<i64>,
        /// Prune other model profiles after active coverage reaches 100%.
        #[arg(long)]
        prune: bool,
        /// Emit a single JSON object with stable fields for scripts.
        #[arg(long)]
        json: bool,
    },
}
