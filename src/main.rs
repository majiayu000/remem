mod ai;
mod context;
mod db;
mod db_query;
mod install;
mod log;
mod mcp;
mod observe;
mod search;
mod summarize;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "remem", about = "Persistent memory for Claude Code")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate context for SessionStart hook (stdout → CLAUDE.md)
    Context {
        /// Working directory (defaults to CWD)
        #[arg(long)]
        cwd: Option<String>,
        /// Session ID
        #[arg(long)]
        session_id: Option<String>,
        /// Use color output
        #[arg(long)]
        color: bool,
    },
    /// Initialize/update session from UserPromptSubmit hook (stdin JSON)
    SessionInit,
    /// Extract observations from PostToolUse hook (stdin JSON)
    Observe,
    /// Stop hook dispatcher: spawn background worker, return immediately
    Summarize,
    /// Background worker: actual summarization (called by Summarize, not by hooks)
    SummarizeWorker,
    /// Flush pending observation queue (batch process with one AI call)
    Flush {
        /// Session ID
        #[arg(long)]
        session_id: String,
        /// Project name
        #[arg(long)]
        project: String,
    },
    /// Run MCP server (stdio transport, long-running)
    Mcp,
    /// Install hooks + MCP to ~/.claude/settings.json
    Install,
    /// Uninstall hooks + MCP from ~/.claude/settings.json
    Uninstall,
    /// 清理旧数据：删除孤立 summary、重复 summary、过期 pending
    Cleanup,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Context { cwd, session_id, color } => {
            let cwd = cwd.unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });
            context::generate_context(&cwd, session_id.as_deref(), color)?;
        }
        Commands::SessionInit => {
            observe::session_init().await?;
        }
        Commands::Observe => {
            observe::observe().await?;
        }
        Commands::Summarize => {
            summarize::summarize().await?;
        }
        Commands::SummarizeWorker => {
            summarize::summarize_worker().await?;
        }
        Commands::Flush { session_id, project } => {
            observe::flush_pending(&session_id, &project).await?;
        }
        Commands::Mcp => {
            mcp::run_mcp_server().await?;
        }
        Commands::Install => {
            install::install()?;
        }
        Commands::Uninstall => {
            install::uninstall()?;
        }
        Commands::Cleanup => {
            let conn = db::open_db()?;
            let orphans = db::cleanup_orphan_summaries(&conn)?;
            let dupes = db::cleanup_duplicate_summaries(&conn)?;
            let stale = db::cleanup_stale_pending(&conn)?;
            println!("清理完成:");
            println!("  孤立 summary (无对应 observation): {} 条", orphans);
            println!("  重复 summary (同 session 多条): {} 条", dupes);
            println!("  过期 pending (超 1 小时): {} 条", stale);
        }
    }

    Ok(())
}
