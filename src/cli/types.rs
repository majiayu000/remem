use clap::{Parser, Subcommand};
use std::path::PathBuf;

pub(super) use crate::install::InstallTarget;

#[derive(Parser)]
#[command(
    name = "remem",
    about = "Persistent memory for Claude Code and Codex",
    version
)]
pub(super) struct Cli {
    #[command(subcommand)]
    pub(super) command: Commands,
}

#[derive(Subcommand)]
pub(super) enum Commands {
    Context {
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        color: bool,
    },
    SessionInit,
    Observe,
    Summarize,
    Worker {
        #[arg(long)]
        once: bool,
    },
    Mcp,
    Install {
        /// Which host(s) to install into.
        #[arg(long, value_enum, default_value = "auto")]
        target: InstallTarget,
        /// Print what would be written without touching disk.
        #[arg(long)]
        dry_run: bool,
    },
    Uninstall {
        /// Which host(s) to uninstall from. Defaults to all known hosts.
        #[arg(long, value_enum, default_value = "auto")]
        target: InstallTarget,
        /// Print what would be removed without touching disk.
        #[arg(long)]
        dry_run: bool,
    },
    Cleanup,
    SyncMemory {
        #[arg(long)]
        cwd: Option<String>,
    },
    Preferences {
        #[command(subcommand)]
        action: PreferenceAction,
    },
    Pending {
        #[command(subcommand)]
        action: PendingAction,
    },
    Status,
    Doctor,
    Search {
        query: String,
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, short = 't')]
        memory_type: Option<String>,
        #[arg(long, short = 'n', default_value = "10")]
        limit: i64,
    },
    Show {
        id: i64,
    },
    Eval {
        #[arg(long, default_value = "eval/golden.json")]
        dataset: String,
        #[arg(long, short = 'k', default_value = "5")]
        k: usize,
    },
    EvalLocal,
    BackfillEntities,
    Encrypt,
    Api {
        #[arg(long, short, default_value = "5567")]
        port: u16,
    },
    Dream {
        #[arg(long, short)]
        project: Option<String>,
        /// Print what would be merged without writing to DB
        #[arg(long)]
        dry_run: bool,
    },
    /// v2 admin commands (backup/reset/import). See SPEC §4.1.
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
    /// v2 import commands (legacy v1 -> v2 best-effort).
    Import {
        #[command(subcommand)]
        action: ImportAction,
    },
}

#[derive(Subcommand)]
pub(in crate::cli) enum PreferenceAction {
    List,
    Add {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        global: bool,
        text: String,
    },
    Remove {
        id: i64,
    },
}

#[derive(Subcommand)]
pub(in crate::cli) enum AdminAction {
    /// Back up the v1 database to a timestamped file.
    Backup {
        /// Output path. Defaults to <data_dir>/backups/remem-v1-<ts>.sqlite.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Drop and re-initialize the v2 database (~/.remem/v2.sqlite).
    /// Requires --confirm-destructive to actually run.
    ResetV2 {
        #[arg(long)]
        confirm_destructive: bool,
    },
}

#[derive(Subcommand)]
pub(in crate::cli) enum ImportAction {
    /// Import v1 memories from a backup sqlite file. Per SPEC §17 D5,
    /// transcripts are not replayed; only the v1 `memories` table is
    /// migrated, with synthesized v2 provenance defaults.
    Legacy {
        /// v1 db path (typically a backup produced by `remem admin backup`).
        #[arg(long)]
        source: PathBuf,
        /// Acknowledge that import is best-effort and skips constraint
        /// violations rather than failing.
        #[arg(long)]
        best_effort: bool,
    },
}

#[derive(Subcommand)]
#[allow(clippy::enum_variant_names)]
pub(in crate::cli) enum PendingAction {
    ListFailed {
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, short = 'n', default_value = "20")]
        limit: i64,
    },
    RetryFailed {
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, short = 'n', default_value = "100")]
        limit: i64,
    },
    PurgeFailed {
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, default_value = "7")]
        older_than_days: i64,
    },
}
