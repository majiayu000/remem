use super::SchemaInvariant;

pub(in crate::migrate) const V081_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::table(
        81,
        "project_identity_aliases",
        "project_identity_alias_events",
    ),
    SchemaInvariant::table(81, "project_identity_aliases", "project_identity_aliases"),
    SchemaInvariant::index(
        81,
        "project_identity_aliases",
        "idx_project_identity_alias_events_path",
    ),
    SchemaInvariant::index(
        81,
        "project_identity_aliases",
        "idx_project_identity_alias_events_target",
    ),
    SchemaInvariant::index(
        81,
        "project_identity_aliases",
        "idx_project_identity_aliases_target_status",
    ),
];
