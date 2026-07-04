mod admin;
mod config_command;
mod embedding;
mod encrypt_state;
mod eval;
mod import;
mod ingest_sessions;
mod maintenance;
mod markdown_archive;
mod memory_policy;
mod model;
mod pending;
mod preferences;
mod query;
mod review;
mod scope_cleanup;
mod shared;
mod usage;
mod user_profile;
mod user_review;
mod user_summary;

pub(super) use admin::run_admin;
pub(super) use config_command::run_config;
pub(super) use embedding::run_embedding;
pub(super) use eval::{
    run_bench, run_eval, run_eval_associative_baseline, run_eval_capacity, run_eval_coding_bench,
    run_eval_e2e, run_eval_extraction, run_eval_gates, run_eval_governance,
    run_eval_graph_decision, run_eval_local, run_eval_weight_grid,
};
pub(super) use import::run_import;
pub(super) use ingest_sessions::run_ingest_sessions_cli;
pub(super) use maintenance::{
    run_cleanup, run_dream, run_encrypt, run_governance, GovernanceCliRequest,
};
pub(super) use markdown_archive::run_export_markdown;
pub(super) use memory_policy::run_memory_action;
pub(super) use model::run_model;
pub(super) use pending::run_pending;
pub(super) use preferences::{run_preferences, run_user};
pub(super) use query::{
    run_backfill_embeddings, run_backfill_entities, run_commit, run_current_state, run_raw,
    run_search, run_show, run_status, run_timeline, run_why, run_workstreams,
};
pub(super) use review::{run_graph_review, run_review};
pub(super) use scope_cleanup::{
    run_archive, run_audit_scope, run_merge_preferences, run_reroute, RerouteCliRequest,
};
pub(super) use usage::run_usage;
pub(super) use user_profile::run_user_profile;
pub(super) use user_review::run_user_review;
pub(super) use user_summary::run_user_summary;
