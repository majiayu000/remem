use anyhow::Result;

use crate::{api, claude_memory, context, db, doctor, install, mcp, observe, summarize, worker};

use super::actions::{
    run_backfill_entities, run_cleanup, run_dream, run_encrypt, run_eval, run_eval_local,
    run_pending, run_preferences, run_search, run_show, run_status,
};
use super::cwd::resolve_cwd_arg;
use super::types::{Cli, Commands};

pub(super) async fn run_cli(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Context {
            cwd,
            session_id,
            color,
        } => {
            if remem_hooks_disabled() {
                return Ok(());
            }
            let cwd = resolve_cwd_arg(cwd);
            context::generate_context(&cwd, session_id.as_deref(), color)?;
        }
        Commands::SessionInit => {
            if remem_hooks_disabled() {
                return Ok(());
            }
            observe::session_init().await?;
        }
        Commands::Observe => {
            if remem_hooks_disabled() {
                return Ok(());
            }
            observe::observe().await?;
        }
        Commands::Summarize => {
            if remem_hooks_disabled() {
                return Ok(());
            }
            summarize::summarize().await?;
        }
        Commands::Worker { once } => worker::run(once, 2000).await?,
        Commands::Mcp => mcp::run_mcp_server().await?,
        Commands::Install { target, dry_run } => install::install(target, dry_run)?,
        Commands::Uninstall { target, dry_run } => install::uninstall(target, dry_run)?,
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
        Commands::Dream { project, dry_run } => {
            run_dream(project.as_deref(), dry_run).await?;
        }
    }

    Ok(())
}

fn remem_hooks_disabled() -> bool {
    std::env::var("REMEM_DISABLE_HOOKS")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}
