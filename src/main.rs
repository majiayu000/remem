use anyhow::Result;
use clap::{Parser, Subcommand};
use remem::{context, db, install, mcp, memory, observe};

#[derive(Parser)]
#[command(name = "remem", about = "Persistent memory for Claude Code")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate context for SessionStart hook (stdout -> CLAUDE.md)
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
    /// Stop hook: auto-summarize session activity and run cleanup
    Summarize,
    /// Run MCP server (stdio transport, long-running)
    Mcp,
    /// Install hooks + MCP to ~/.claude/settings.json
    Install,
    /// Uninstall hooks + MCP from ~/.claude/settings.json
    Uninstall,
    /// Run data cleanup (old events + stale memories)
    Cleanup,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Context {
            cwd,
            session_id,
            color,
        } => {
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
            run_summarize()?;
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
            run_cleanup()?;
        }
    }

    Ok(())
}

/// Stop hook: pure SQL session summary (no LLM).
fn run_summarize() -> Result<()> {
    let input = std::io::read_to_string(std::io::stdin())?;
    let hook: serde_json::Value = serde_json::from_str(&input)?;

    let session_id = hook["session_id"].as_str().unwrap_or("unknown").to_string();
    let cwd = hook["cwd"].as_str().unwrap_or(".").to_string();
    let project = db::project_from_cwd(&cwd);

    let conn = db::open_db()?;

    // Count session activity
    let event_count = memory::count_session_events(&conn, &session_id)?;
    let memory_count = memory::count_session_memories(&conn, &session_id)?;

    remem::log::info(
        "summarize",
        &format!(
            "session={} events={} memories={}",
            session_id, event_count, memory_count
        ),
    );

    // If Claude didn't save any memories, generate auto activity summary
    if memory_count == 0 && event_count > 0 {
        let files = memory::get_session_files_modified(&conn, &session_id)?;
        let files_str = if files.is_empty() {
            "none".to_string()
        } else {
            files.join(", ")
        };
        let summary = format!(
            "Session activity: {} events, modified files: [{}]",
            event_count, files_str
        );

        memory::insert_memory(
            &conn,
            Some(&session_id),
            &project,
            None,
            &format!("Session {}", &session_id[..session_id.len().min(8)]),
            &summary,
            "session_activity",
            None,
        )?;

        remem::log::info(
            "summarize",
            &format!("auto-saved session_activity: {summary}"),
        );
    }

    // Cleanup old data
    let events_deleted = memory::cleanup_old_events(&conn, 30)?;
    let memories_archived = memory::archive_stale_memories(&conn, 180)?;

    if events_deleted > 0 || memories_archived > 0 {
        remem::log::info(
            "summarize",
            &format!(
                "cleanup: deleted {} old events, archived {} stale memories",
                events_deleted, memories_archived
            ),
        );
    }

    // Workstream lifecycle
    let paused = remem::workstream::auto_pause_inactive(&conn, &project, 7)?;
    let abandoned = remem::workstream::auto_abandon_inactive(&conn, &project, 30)?;

    if paused > 0 || abandoned > 0 {
        remem::log::info(
            "summarize",
            &format!(
                "workstream lifecycle: paused={}, abandoned={}",
                paused, abandoned
            ),
        );
    }

    Ok(())
}

/// Cleanup old events and archive stale memories.
fn run_cleanup() -> Result<()> {
    let conn = db::open_db()?;
    let events_deleted = memory::cleanup_old_events(&conn, 30)?;
    let memories_archived = memory::archive_stale_memories(&conn, 180)?;
    println!("Cleanup complete:");
    println!("  Old events deleted (>30 days): {}", events_deleted);
    println!(
        "  Stale memories archived (>180 days): {}",
        memories_archived
    );
    Ok(())
}
