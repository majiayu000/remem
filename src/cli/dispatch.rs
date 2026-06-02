use anyhow::Result;

use crate::{api, context, db, doctor, install, mcp, observe, summarize, worker};

use super::actions::{
    run_admin, run_archive, run_audit_scope, run_backfill_entities, run_cleanup, run_commit,
    run_dream, run_encrypt, run_eval, run_eval_e2e, run_eval_governance, run_eval_local,
    run_governance, run_import, run_memory_cleanup, run_merge_preferences, run_pending,
    run_preferences, run_raw, run_reroute, run_review, run_search, run_show, run_status, run_usage,
    run_why, GovernanceCliRequest, RerouteCliRequest,
};
use super::cwd::resolve_cwd_arg;
use super::types::{Cli, Commands, ContextGateAction, MemoryAction};

#[path = "actions/context_gate.rs"]
mod context_gate;
use context_gate::run_context_gate_status;

pub(super) async fn run_cli(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Context {
            cwd,
            session_id,
            host,
            color,
            debug,
            force,
            gate,
        } => {
            if remem_hooks_disabled() {
                return Ok(());
            }
            context::generate_context_from_cli(cwd, session_id, color, host, debug, force, gate)?;
        }
        Commands::ContextGate { action } => match action {
            ContextGateAction::Status {
                project,
                session,
                limit,
                json,
            } => run_context_gate_status(project.as_deref(), session.as_deref(), limit, json)?,
        },
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
            context::claude_memory::sync_to_claude_memory(&cwd, &project)?;
        }
        Commands::Preferences { action } => run_preferences(action)?,
        Commands::Memory { action } => match action {
            MemoryAction::Cleanup {
                cwd,
                cleanup_type,
                all_types,
                dry_run,
                plan_out,
                apply,
                plan,
                json,
            } => run_memory_cleanup(
                cwd.as_deref(),
                cleanup_type,
                all_types,
                dry_run,
                plan_out.as_deref(),
                apply,
                plan.as_deref(),
                json,
            )?,
        },
        Commands::Pending { action } => run_pending(action)?,
        Commands::Review { action } => run_review(action)?,
        Commands::Govern {
            project,
            action,
            reason,
            actor,
            query,
            memory_type,
            status,
            limit,
            offset,
            from_file,
            read_stdin,
            confirm_destructive,
            dry_run,
            json,
            ids,
        } => run_governance(GovernanceCliRequest {
            project: project.as_deref(),
            action,
            reason: reason.as_deref(),
            actor: actor.as_deref(),
            query: query.as_deref(),
            memory_type: memory_type.as_deref(),
            status: status.as_deref(),
            limit,
            offset,
            from_file: from_file.as_deref(),
            read_stdin,
            confirm_destructive,
            dry_run,
            json,
            ids: &ids,
        })?,
        Commands::AuditScope {
            project,
            limit,
            json,
        } => run_audit_scope(project.as_deref(), limit, json)?,
        Commands::Reroute {
            refs,
            ids,
            owner_scope,
            owner_key,
            target_project,
            clear_target_project,
            topic_domain,
            context_class,
            confidence,
            reason,
            confirm,
            dry_run,
            json,
        } => run_reroute(RerouteCliRequest {
            refs: &refs,
            ids: &ids,
            owner_scope: &owner_scope,
            owner_key: &owner_key,
            target_project: target_project.as_deref(),
            clear_target_project,
            topic_domain: topic_domain.as_deref(),
            context_class: context_class.as_deref(),
            confidence,
            reason: reason.as_deref(),
            confirm,
            dry_run,
            json,
        })?,
        Commands::Archive {
            refs,
            ids,
            reason,
            confirm,
            dry_run,
            json,
        } => run_archive(&refs, &ids, reason.as_deref(), confirm, dry_run, json)?,
        Commands::MergePreferences {
            project,
            dry_run,
            confirm,
            json,
        } => run_merge_preferences(project.as_deref(), dry_run, confirm, json)?,
        Commands::Usage {
            project,
            days,
            weeks,
        } => run_usage(project.as_deref(), days, weeks)?,
        Commands::Status { json } => run_status(json)?,
        Commands::Doctor { json, quiet } => {
            let outcome = doctor::run_doctor(doctor::DoctorOptions { json, quiet })?;
            let code = outcome.exit_code();
            if code != 0 {
                std::process::exit(code);
            }
        }
        Commands::Search {
            query,
            project,
            memory_type,
            limit,
            offset,
            branch,
            include_stale,
            multi_hop,
            explain,
            json,
        } => run_search(
            &query,
            project.as_deref(),
            memory_type.as_deref(),
            limit,
            offset,
            branch.as_deref(),
            include_stale,
            multi_hop,
            explain,
            json,
        )?,
        Commands::Raw { action } => run_raw(action)?,
        Commands::Commit { action } => run_commit(action)?,
        Commands::Show { id, json } => run_show(id, json)?,
        Commands::Why {
            id,
            project,
            branch,
        } => run_why(id, project.as_deref(), branch.as_deref())?,
        Commands::Eval { dataset, k } => run_eval(&dataset, k)?,
        Commands::EvalE2e {
            k,
            json,
            keep_data_dir,
        } => run_eval_e2e(k, json, keep_data_dir).await?,
        Commands::EvalGovernance { k, json } => run_eval_governance(k, json)?,
        Commands::EvalLocal => run_eval_local()?,
        Commands::BackfillEntities => run_backfill_entities()?,
        Commands::Encrypt => run_encrypt()?,
        Commands::Api { port } => api::run_api_server(port).await?,
        Commands::Dream { project, dry_run } => {
            run_dream(project.as_deref(), dry_run).await?;
        }
        Commands::Admin { action } => run_admin(action)?,
        Commands::Import { action } => run_import(action)?,
    }

    Ok(())
}

fn remem_hooks_disabled() -> bool {
    std::env::var("REMEM_DISABLE_HOOKS")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}
