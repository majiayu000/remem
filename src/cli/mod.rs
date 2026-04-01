mod actions;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::{api, claude_memory, context, db, doctor, install, mcp, observe, summarize, worker};
use actions::{
    run_backfill_entities, run_cleanup, run_encrypt, run_eval, run_eval_local, run_pending,
    run_preferences, run_search, run_show, run_status,
};

#[derive(Parser)]
#[command(name = "remem", about = "Persistent memory for Claude Code", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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
}

#[derive(Subcommand)]
pub(super) enum PreferenceAction {
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
pub(super) enum PendingAction {
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

fn resolve_cwd_arg(cwd: Option<String>) -> String {
    cwd.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    })
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Context {
            cwd,
            session_id,
            color,
        } => {
            let cwd = resolve_cwd_arg(cwd);
            context::generate_context(&cwd, session_id.as_deref(), color)?;
        }
        Commands::SessionInit => observe::session_init().await?,
        Commands::Observe => observe::observe().await?,
        Commands::Summarize => summarize::summarize().await?,
        Commands::Worker { once } => worker::run(once, 2000).await?,
        Commands::Mcp => mcp::run_mcp_server().await?,
        Commands::Install => install::install()?,
        Commands::Uninstall => install::uninstall()?,
        Commands::Cleanup => run_cleanup()?,
        Commands::SyncMemory { cwd } => {
            let cwd = resolve_cwd_arg(cwd);
            let project = db::project_from_cwd(&cwd);
            claude_memory::sync_to_claude_memory(&cwd, &project)?;
        }
        Commands::Preferences { action } => run_preferences(action)?,
        Commands::Pending { action } => run_pending(action)?,
        Commands::Status => run_status()?,
        Commands::Doctor => doctor::run_doctor()?,
        Commands::Search {
            query,
            project,
            memory_type,
            limit,
        } => run_search(&query, project.as_deref(), memory_type.as_deref(), limit)?,
        Commands::Show { id } => run_show(id)?,
        Commands::Eval { dataset, k } => run_eval(&dataset, k)?,
        Commands::EvalLocal => run_eval_local()?,
        Commands::BackfillEntities => run_backfill_entities()?,
        Commands::Encrypt => run_encrypt()?,
        Commands::Api { port } => api::run_api_server(port).await?,
    }

    Ok(())
}
