use anyhow::Result;

use crate::{api, claude_memory, context, db, doctor, install, mcp, observe, summarize, worker};

use super::actions::{
    run_backfill_entities, run_cleanup, run_encrypt, run_eval, run_eval_local, run_pending,
    run_preferences, run_search, run_show, run_status,
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
