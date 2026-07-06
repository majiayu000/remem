use anyhow::Result;

use crate::{api, context, db, doctor, install, mcp, observe, summarize, worker};

use super::actions::{
    run_admin, run_archive, run_audit_scope, run_backfill_embeddings, run_backfill_entities,
    run_bench, run_cleanup, run_commit, run_config, run_current_state, run_dream, run_embedding,
    run_encrypt, run_eval, run_eval_associative_baseline, run_eval_capacity, run_eval_coding_bench,
    run_eval_e2e, run_eval_extraction, run_eval_gates, run_eval_governance,
    run_eval_graph_decision, run_eval_local, run_eval_provider_comparison, run_eval_weight_grid,
    run_export, run_governance, run_graph_review, run_import, run_ingest_sessions_cli,
    run_memory_action, run_merge_preferences, run_model, run_pending, run_preferences,
    run_procedures, run_raw, run_reroute, run_review, run_search, run_show, run_status,
    run_timeline, run_usage, run_user, run_why, run_workstreams, GovernanceCliRequest,
    RerouteCliRequest,
};
use super::cwd::resolve_cwd_arg;
use super::types::{Cli, Commands, ContextGateAction};

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
        Commands::Config { action } => run_config(action)?,
        Commands::Model { action } => run_model(action).await?,
        Commands::Embedding { action } => run_embedding(action)?,
        Commands::SessionInit { host } => {
            if remem_hooks_disabled() {
                return Ok(());
            }
            observe::session_init(host.as_deref()).await?;
        }
        Commands::Observe { host } => {
            if remem_hooks_disabled() {
                return Ok(());
            }
            observe::observe(host.as_deref()).await?;
        }
        Commands::Summarize { host, profile } => {
            if remem_hooks_disabled() {
                return Ok(());
            }
            summarize::summarize(host.as_deref(), profile.as_deref()).await?;
        }
        Commands::Worker { once } => worker::run(once, 2000).await?,
        Commands::Mcp => mcp::run_mcp_server().await?,
        Commands::Install {
            target,
            hooks_only,
            dry_run,
        } => install::install(target, dry_run, hooks_only)?,
        Commands::Uninstall { target, dry_run } => install::uninstall(target, dry_run)?,
        Commands::Cleanup {
            dry_run,
            json,
            archived_failures,
        } => run_cleanup(dry_run, json, archived_failures)?,
        Commands::SyncMemory { cwd } => {
            let cwd = resolve_cwd_arg(cwd);
            let project = db::project_from_cwd(&cwd);
            let conn = db::open_db()?;
            context::claude_memory::sync_to_claude_memory(&conn, &cwd, &project)?;
        }
        Commands::Preferences { action } => run_preferences(action)?,
        Commands::User { action } => run_user(action)?,
        Commands::Memory { action } => run_memory_action(action)?,
        Commands::Pending { action } => run_pending(action)?,
        Commands::Review { action } => run_review(action)?,
        Commands::GraphReview { action } => run_graph_review(action)?,
        Commands::Procedures { action } => run_procedures(action)?,
        Commands::Govern {
            project,
            action,
            acknowledge_pattern,
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
            acknowledge_pattern: acknowledge_pattern.as_deref(),
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
            include_suppressed,
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
            include_suppressed,
            multi_hop,
            explain,
            json,
        )?,
        Commands::Current {
            state_key,
            project,
            memory_type,
            owner_scope,
            owner_key,
            as_of_epoch,
            json,
        } => run_current_state(
            &state_key,
            project.as_deref(),
            owner_scope.as_deref(),
            owner_key.as_deref(),
            memory_type.as_deref(),
            as_of_epoch,
            json,
        )?,
        Commands::Raw { action } => run_raw(action)?,
        Commands::Timeline { action } => run_timeline(action)?,
        Commands::Workstreams { action } => run_workstreams(action)?,
        Commands::Commit { action } => run_commit(action)?,
        Commands::Show { id, json } => run_show(id, json)?,
        Commands::Why {
            id,
            project,
            branch,
        } => run_why(id, project.as_deref(), branch.as_deref())?,
        Commands::Bench { action } => run_bench(action)?,
        Commands::Eval { dataset, k, json } => run_eval(&dataset, k, json)?,
        Commands::EvalE2e {
            k,
            json,
            keep_data_dir,
        } => run_eval_e2e(k, json, keep_data_dir).await?,
        Commands::EvalGovernance { k, json } => run_eval_governance(k, json)?,
        Commands::EvalExtraction(args) => {
            run_eval_extraction(&args.corpus, &args.baseline, args.json, args.check_baseline)?
        }
        Commands::EvalProviderComparison(args) => run_eval_provider_comparison(args)?,
        Commands::EvalGraphDecision(args) => {
            run_eval_graph_decision(&args.dataset, args.k, &args.json_out, args.json)?
        }
        Commands::EvalAssociativeBaseline(args) => {
            run_eval_associative_baseline(&args.dataset, args.k, &args.json_out, args.json)?
        }
        Commands::EvalCapacity(args) => run_eval_capacity(args)?,
        Commands::EvalWeightGrid(args) => {
            run_eval_weight_grid(&args.dataset, args.k, &args.json_out, args.json)?
        }
        Commands::EvalGates(args) => run_eval_gates(
            &args.baseline,
            &args.thresholds,
            &args.golden_dataset,
            args.json_out.as_deref(),
            args.json,
            args.simulate_golden_regression,
            args.simulate_capacity_regression,
        )?,
        Commands::EvalCodingBench(args) => run_eval_coding_bench(args)?,
        Commands::EvalLocal => run_eval_local()?,
        Commands::BackfillEntities => run_backfill_entities()?,
        Commands::BackfillEmbeddings { limit, batch_size } => {
            run_backfill_embeddings(limit, batch_size)?
        }
        Commands::Encrypt { rekey_raw } => run_encrypt(rekey_raw)?,
        Commands::Api { port } => api::run_api_server(port).await?,
        Commands::Dream {
            project,
            profile,
            dry_run,
        } => {
            run_dream(project.as_deref(), profile.as_deref(), dry_run).await?;
        }
        Commands::Admin { action } => run_admin(action)?,
        Commands::Import {
            action,
            pack,
            dry_run,
        } => {
            let project = pack
                .as_ref()
                .map(|_| db::project_from_cwd(&resolve_cwd_arg(None)));
            run_import(action, pack.as_deref(), dry_run, project.as_deref())?;
        }
        Commands::IngestSessions { roots, since, json } => {
            let summary = run_ingest_sessions_cli(&roots, since.as_deref(), json)?;
            let code = summary.exit_code();
            if code != 0 {
                std::process::exit(code);
            }
        }
        Commands::Export(args) => {
            let project = args
                .project
                .as_deref()
                .map(str::to_string)
                .unwrap_or_else(|| db::project_from_cwd(&resolve_cwd_arg(None)));
            run_export(args, &project)?;
        }
    }

    Ok(())
}

fn remem_hooks_disabled() -> bool {
    std::env::var("REMEM_DISABLE_HOOKS")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}
