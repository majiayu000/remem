use super::SchemaInvariant;

pub(in crate::migrate) const V085_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::table(85, "legacy_pending_bridge_state", "legacy_surface_state"),
    SchemaInvariant::column(
        85,
        "legacy_pending_bridge_state",
        "legacy_surface_state",
        "state",
    ),
    SchemaInvariant::column(
        85,
        "legacy_pending_bridge_state",
        "legacy_surface_state",
        "residual_count",
    ),
];
