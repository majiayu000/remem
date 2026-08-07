use super::SchemaInvariant;

pub(in crate::migrate) const V079_SCHEMA_INVARIANTS: &[SchemaInvariant] =
    &[SchemaInvariant::column(
        79,
        "candidate_spo_facts",
        "memory_candidates",
        "facts",
    )];
