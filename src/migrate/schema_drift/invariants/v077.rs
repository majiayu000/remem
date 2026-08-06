use super::SchemaInvariant;

pub(in crate::migrate) const V077_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::column(
        77,
        "dream_backfill",
        "dream_quarantine_artifacts",
        "backfill_memory_id",
    ),
    SchemaInvariant::index(77, "dream_backfill", "idx_dream_quarantine_backfill_memory"),
    SchemaInvariant::trigger(
        77,
        "dream_backfill",
        "dream_quarantine_artifacts_backfill_merge_only",
    ),
    SchemaInvariant::trigger(
        77,
        "dream_backfill",
        "dream_quarantine_artifacts_backfill_immutable",
    ),
];
