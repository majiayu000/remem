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
    Review {
        #[command(subcommand)]
        action: ReviewAction,
    },
    Usage {
        /// Restrict usage totals to one project path.
        #[arg(long, short)]
        project: Option<String>,
        /// Number of daily buckets to show.
        #[arg(long, default_value = "14")]
        days: i64,
        /// Number of weekly buckets to show.
        #[arg(long, default_value = "8")]
        weeks: i64,
    },
    Status,
    Doctor {
        /// Emit a single JSON object with per-check status. Stable shape;
        /// fields: `version`, `status`, `fails`, `warns`, `checks[]`.
        #[arg(long)]
        json: bool,
        /// Suppress human-readable output. Useful when only the exit code
        /// matters (CI/scripts). Has no effect when `--json` is set.
        #[arg(long, short)]
        quiet: bool,
    },
    Search {
        query: String,
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, short = 't')]
        memory_type: Option<String>,
        #[arg(long, short = 'n', default_value = "10")]
        limit: i64,
        #[arg(long, default_value = "0")]
        offset: i64,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        include_stale: bool,
        #[arg(long)]
        multi_hop: bool,
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
    /// Admin commands for database backup and schema reset.
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
    /// Import commands for moving older backup rows into the schema database.
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
    /// Back up the existing remem database to a timestamped file.
    Backup {
        /// Output path. Defaults to <data_dir>/backups/remem-backup-<ts>.sqlite.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Drop and re-initialize the schema database (~/.remem/schema.sqlite).
    /// Requires --confirm-destructive to actually run.
    #[command(name = "reset-schema")]
    ResetSchema {
        #[arg(long)]
        confirm_destructive: bool,
    },
}

#[derive(Subcommand)]
pub(in crate::cli) enum ImportAction {
    /// Import memories from an older backup sqlite file. Transcripts are not
    /// replayed; only the old `memories` table is imported, with synthesized
    /// provenance defaults.
    Backup {
        /// Backup sqlite path produced by `remem admin backup`.
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
        #[arg(long)]
        dry_run: bool,
    },
    PurgeFailed {
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, default_value = "7")]
        older_than_days: i64,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub(in crate::cli) enum ReviewAction {
    List {
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, short = 'n', default_value = "20")]
        limit: i64,
    },
    Approve {
        id: i64,
    },
    Discard {
        id: i64,
    },
    Edit {
        id: i64,
        #[arg(long)]
        text: Option<String>,
        #[arg(long = "topic-key")]
        topic_key: Option<String>,
        #[arg(long = "type")]
        memory_type: Option<String>,
        #[arg(long)]
        scope: Option<String>,
    },
}
