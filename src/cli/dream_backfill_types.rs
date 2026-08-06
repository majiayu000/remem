use clap::Args;

#[derive(Args, Debug, Clone, Default)]
pub(in crate::cli) struct DreamBackfillArgs {
    /// Report what would change without writing (default behavior).
    #[arg(long)]
    pub(in crate::cli) dry_run: bool,
    /// Execute the quarantine and trust-class writes. Quarantine ledger rows
    /// are immutable, so this cannot be undone.
    #[arg(long)]
    pub(in crate::cli) apply: bool,
    /// Require an explicit dry-run digest before applying irreversible writes.
    #[arg(long, value_name = "SHA256")]
    pub(in crate::cli) expect_plan_digest: Option<String>,
    /// Emit a single JSON object with stable fields for scripts.
    #[arg(long)]
    pub(in crate::cli) json: bool,
}
