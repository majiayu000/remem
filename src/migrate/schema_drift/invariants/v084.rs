use super::SchemaInvariant;

pub(in crate::migrate) const V084_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::table(84, "legacy_pending_bridge_state", "legacy_surface_state"),
    SchemaInvariant::column(
        84,
        "legacy_pending_bridge_state",
        "legacy_surface_state",
        "state",
    ),
    SchemaInvariant::column(
        84,
        "legacy_pending_bridge_state",
        "legacy_surface_state",
        "residual_count",
    ),
];
