use super::SchemaInvariant;

pub(in crate::migrate) const V075_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::column(75, "automatic_cleanup", "events", "retention_class"),
    SchemaInvariant::index(75, "automatic_cleanup", "idx_events_retention_created"),
    SchemaInvariant::index(75, "automatic_cleanup", "idx_api_mutation_requests_audit"),
    SchemaInvariant::index(75, "automatic_cleanup", "idx_memories_cleanup_expiry"),
    SchemaInvariant::index(75, "automatic_cleanup", "idx_memories_cleanup_archive"),
    SchemaInvariant::index(
        75,
        "automatic_cleanup",
        "idx_workstreams_cleanup_inactivity",
    ),
    SchemaInvariant::index(75, "automatic_cleanup", "idx_observations_cleanup_sources"),
    SchemaInvariant::index(
        75,
        "automatic_cleanup",
        "idx_compressed_sources_cleanup_age",
    ),
    SchemaInvariant::trigger(
        75,
        "automatic_cleanup",
        "events_preserve_api_mutation_audit",
    ),
    SchemaInvariant::table(75, "automatic_cleanup", "maintenance_runs"),
    SchemaInvariant::column(75, "automatic_cleanup", "maintenance_runs", "job_id"),
    SchemaInvariant::column(75, "automatic_cleanup", "maintenance_runs", "trigger"),
    SchemaInvariant::column(
        75,
        "automatic_cleanup",
        "maintenance_runs",
        "policy_version",
    ),
    SchemaInvariant::column(
        75,
        "automatic_cleanup",
        "maintenance_runs",
        "started_at_epoch",
    ),
    SchemaInvariant::column(
        75,
        "automatic_cleanup",
        "maintenance_runs",
        "finished_at_epoch",
    ),
    SchemaInvariant::column(75, "automatic_cleanup", "maintenance_runs", "outcome"),
    SchemaInvariant::column(75, "automatic_cleanup", "maintenance_runs", "counts_json"),
    SchemaInvariant::column(75, "automatic_cleanup", "maintenance_runs", "error"),
    SchemaInvariant::index(
        75,
        "automatic_cleanup",
        "idx_maintenance_runs_trigger_outcome_finished",
    ),
    SchemaInvariant::index(75, "automatic_cleanup", "idx_maintenance_runs_job"),
    SchemaInvariant::index(75, "automatic_cleanup", "idx_jobs_active_cleanup_unique"),
];
