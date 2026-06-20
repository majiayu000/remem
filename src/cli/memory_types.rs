use clap::{Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Subcommand)]
pub(in crate::cli) enum MemoryAction {
    /// Suppress a memory, claim, topic, entity, pattern, or summary target.
    Suppress {
        target: String,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        actor: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Revoke a suppression by id or by target.
    Unsuppress {
        target: String,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        actor: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Record relevance feedback without changing ranking by default.
    Feedback {
        target: String,
        #[arg(long)]
        value: String,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        context_injection_item_id: Option<i64>,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Inspect suppression policy rows.
    Suppressions {
        #[command(subcommand)]
        action: MemorySuppressionsAction,
    },
    /// Build or apply a dry-run-first memory cleanup plan.
    Cleanup {
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long = "type", value_enum)]
        cleanup_type: Option<MemoryCleanupType>,
        #[arg(long)]
        all_types: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        plan_out: Option<PathBuf>,
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        plan: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(in crate::cli) enum MemorySuppressionsAction {
    List {
        #[arg(long)]
        include_inactive: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(in crate::cli) enum MemoryCleanupType {
    Preference,
}

impl MemoryCleanupType {
    pub(in crate::cli) fn as_str(self) -> &'static str {
        match self {
            Self::Preference => "preference",
        }
    }
}
