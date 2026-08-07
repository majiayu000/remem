use super::SchemaInvariant;

pub(in crate::migrate) const V080_SCHEMA_INVARIANTS: &[SchemaInvariant] =
    &[SchemaInvariant::column(
        80,
        "candidate_outcome",
        "memory_candidates",
        "outcome",
    )];
