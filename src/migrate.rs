mod content_identity;
mod dry_run;
mod git_commit_files;
mod run;
mod schema_drift;
mod state;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_automatic_cleanup;
#[cfg(test)]
mod tests_capture_git_evidence;
#[cfg(test)]
mod tests_compression_provenance;
#[cfg(test)]
mod tests_content_identity;
#[cfg(test)]
mod tests_convergence;
#[cfg(test)]
mod tests_dream_poisoning;
#[cfg(test)]
mod tests_event_capture_projection;
#[cfg(test)]
mod tests_fast_path;
#[cfg(test)]
mod tests_job_queue_atomicity;
#[cfg(test)]
mod tests_legacy_pending_bridge;
#[cfg(test)]
mod tests_legacy_summary;
#[cfg(test)]
mod tests_memory_embeddings;
#[cfg(test)]
mod tests_memory_usage;
#[cfg(test)]
mod tests_preference_rules;
#[cfg(test)]
mod tests_project_identity_aliases;
#[cfg(test)]
mod tests_raw_session_identity;
#[cfg(test)]
mod tests_retrieval_enrichment;
#[cfg(test)]
mod tests_schema;
#[cfg(test)]
mod tests_schema_drift;
#[cfg(test)]
mod tests_session_summary_poisoning;
#[cfg(test)]
mod tests_staleness_index;
#[cfg(test)]
mod tests_user_context;
#[cfg(test)]
mod tests_workstream_identity;
mod transition;
mod types;

pub(crate) use dry_run::dry_run_pending;
pub(crate) use run::ensure_schema_current;
pub use run::run_migrations;
pub(crate) use schema_drift::validate_schema_invariants;
#[cfg(test)]
pub(crate) use types::MIGRATIONS;

pub(crate) fn latest_schema_version() -> i64 {
    types::MIGRATIONS
        .last()
        .map(|migration| migration.version)
        .unwrap_or(0)
}
