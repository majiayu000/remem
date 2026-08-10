use super::SchemaInvariant;

pub(in crate::migrate) const V082_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::table(
        82,
        "project_identity_aliases",
        "project_identity_alias_events",
    ),
    SchemaInvariant::table(82, "project_identity_aliases", "project_identity_aliases"),
    SchemaInvariant::index(
        82,
        "project_identity_aliases",
        "idx_project_identity_alias_events_path",
    ),
    SchemaInvariant::index(
        82,
        "project_identity_aliases",
        "idx_project_identity_alias_events_target",
    ),
    SchemaInvariant::index(
        82,
        "project_identity_aliases",
        "idx_project_identity_aliases_target_status",
    ),
];
