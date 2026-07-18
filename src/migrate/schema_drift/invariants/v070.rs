use super::SchemaInvariant;

pub(in crate::migrate) const V070_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::column(70, "web_console_governance", "memory_candidates", "version"),
    SchemaInvariant::column(70, "web_console_governance", "memories", "version"),
    SchemaInvariant::column(
        70,
        "web_console_governance",
        "memories",
        "web_archive_operation_id",
    ),
    SchemaInvariant::table(70, "web_console_governance", "api_mutation_requests"),
    SchemaInvariant::index(
        70,
        "web_console_governance",
        "idx_api_mutation_requests_resource",
    ),
    SchemaInvariant::trigger(
        70,
        "web_console_governance",
        "memory_candidates_web_version",
    ),
    SchemaInvariant::trigger(70, "web_console_governance", "memories_web_version"),
    SchemaInvariant::trigger(
        70,
        "web_console_governance",
        "memories_clear_web_archive_marker",
    ),
];
