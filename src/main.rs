mod ai;
mod context;
mod db;
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
    /// Generate session summary from Stop hook (stdin JSON)
    Summarize,
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
        Commands::Flush { session_id, project } => {
            observe::flush_pending(&session_id, &project).await?;
        }
        Commands::Mcp => {
            mcp::run_mcp_server().await?;
        }
    }

    Ok(())
}
