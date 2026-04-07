use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "remem", about = "Persistent memory for Claude Code", version)]
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
    Install,
    Uninstall,
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
