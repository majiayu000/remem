use anyhow::Result;
use clap::{Parser, Subcommand};
use remem::{
    claude_memory, context, db, install, mcp, memory, observe, preference, summarize, worker,
};

#[derive(Parser)]
#[command(name = "remem", about = "Persistent memory for Claude Code", version)]
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
    /// Stop hook: enqueue summary job and trigger worker
    Summarize,
    /// Background worker: process queued jobs (summary/compress/flush)
    Worker {
        /// Run one cycle then exit (for hook-triggered workers)
        #[arg(long)]
        once: bool,
    },
    /// Run MCP server (stdio transport, long-running)
    Mcp,
    /// Install hooks + MCP to ~/.claude/settings.json
    Install,
    /// Uninstall hooks + MCP from ~/.claude/settings.json
    Uninstall,
    /// Run data cleanup (old events + stale memories)
    Cleanup,
    /// Sync session summaries to Claude Code native memory directory
    SyncMemory {
        /// Working directory
        #[arg(long)]
        cwd: Option<String>,
    },
    /// Manage preferences
    Preferences {
        #[command(subcommand)]
        action: PreferenceAction,
    },
    /// Show system status and statistics
    Status,
}

#[derive(Subcommand)]
enum PreferenceAction {
    /// List all preferences
    List,
    /// Add a new preference
    Add {
        /// Project name (defaults to current directory)
        #[arg(long)]
        project: Option<String>,
        /// Preference text
        text: String,
    },
    /// Remove a preference by ID
    Remove {
        /// Preference ID
        id: i64,
    },
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
            summarize::summarize().await?;
        }
        Commands::Worker { once } => {
            worker::run(once, 2000).await?;
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
        Commands::SyncMemory { cwd } => {
            let cwd = cwd.unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });
            let project = db::project_from_cwd(&cwd);
            claude_memory::sync_to_claude_memory(&cwd, &project)?;
        }
        Commands::Preferences { action } => {
            run_preferences(action)?;
        }
        Commands::Status => {
            run_status()?;
        }
    }

    Ok(())
}

fn resolve_cwd_project() -> (String, String) {
    let cwd = std::env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let project = db::project_from_cwd(&cwd);
    (cwd, project)
}

fn run_preferences(action: PreferenceAction) -> Result<()> {
    let conn = db::open_db()?;
    let (_, default_project) = resolve_cwd_project();

    match action {
        PreferenceAction::List => {
            preference::list_preferences(&conn, &default_project)?;
        }
        PreferenceAction::Add { project, text } => {
            let proj = project.unwrap_or(default_project);
            let id = preference::add_preference(&conn, &proj, &text)?;
            println!("Preference added (id={}) for project '{}'", id, proj);
        }
        PreferenceAction::Remove { id } => {
            if preference::remove_preference(&conn, id)? {
                println!("Preference {} archived.", id);
            } else {
                println!("Preference {} not found or not a preference type.", id);
            }
        }
    }
    Ok(())
}

/// Show system status and statistics.
fn run_status() -> Result<()> {
    let conn = db::open_db()?;
    let db_path = db::db_path();
    let db_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);

    let version = env!("CARGO_PKG_VERSION");

    let memory_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE status = 'active'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let observation_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM observations WHERE status = 'active'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let session_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM session_summaries", [], |r| r.get(0))
        .unwrap_or(0);
    let pending_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pending_observations", [], |r| {
            r.get(0)
        })
        .unwrap_or(0);

    // Today's stats
    let today_start = chrono::Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|dt| dt.and_utc().timestamp())
        .unwrap_or(0);
    let today_memories: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE created_at_epoch >= ?1",
            rusqlite::params![today_start],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let today_observations: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM observations WHERE created_at_epoch >= ?1",
            rusqlite::params![today_start],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Top projects
    let mut top_projects: Vec<(String, i64)> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT project, COUNT(*) as cnt FROM memories WHERE status = 'active' \
         GROUP BY project ORDER BY cnt DESC LIMIT 5",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        }) {
            for row in rows.flatten() {
                top_projects.push(row);
            }
        }
    }

    println!("remem v{}", version);
    println!(
        "Database: {} ({:.1} MB)",
        db_path.display(),
        db_size as f64 / 1_048_576.0
    );
    println!();
    println!("  Memories:      {:>6}", memory_count);
    println!("  Observations:  {:>6}", observation_count);
    println!("  Sessions:      {:>6}", session_count);
    println!("  Pending:       {:>6}", pending_count);
    println!();
    println!("Today:");
    println!("  New memories:      {:>4}", today_memories);
    println!("  New observations:  {:>4}", today_observations);

    if !top_projects.is_empty() {
        println!();
        println!("Top projects:");
        for (proj, count) in &top_projects {
            println!("  {:>4}  {}", count, proj);
        }
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
