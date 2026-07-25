use super::SchemaInvariant;

macro_rules! enrichment_column {
    ($column:literal) => {
        SchemaInvariant::column(72, "memory_retrieval_enrichment", "memories", $column)
    };
}

macro_rules! compatibility_column {
    ($column:literal) => {
        SchemaInvariant::column(
            72,
            "memory_retrieval_enrichment",
            "retrieval_enrichment_compatibility",
            $column,
        )
    };
}

pub(in crate::migrate) const V072_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    enrichment_column!("search_context_enrichment_version"),
    enrichment_column!("search_context_security_policy_version"),
    enrichment_column!("search_context_source_hash"),
    enrichment_column!("search_context_fallback_source_hash"),
    enrichment_column!("search_context_index_hash"),
    enrichment_column!("search_context_enrichment_attempt"),
    enrichment_column!("search_context_lease_owner"),
    enrichment_column!("search_context_lease_expires_at_epoch"),
    enrichment_column!("search_context_claimed_source_hash"),
    enrichment_column!("search_context_claimed_enrichment_version"),
    enrichment_column!("search_context_claimed_security_policy_version"),
    enrichment_column!("search_context_failure_count"),
    enrichment_column!("search_context_next_retry_at_epoch"),
    enrichment_column!("search_context_last_error_code"),
    SchemaInvariant::table(
        72,
        "memory_retrieval_enrichment",
        "retrieval_enrichment_compatibility",
    ),
    compatibility_column!("min_security_policy_version"),
    compatibility_column!("compatibility_epoch"),
    compatibility_column!("target_security_policy_version"),
    compatibility_column!("convergence_state"),
    SchemaInvariant::trigger(
        72,
        "memory_retrieval_enrichment",
        "retrieval_enrichment_compatibility_bu",
    ),
    SchemaInvariant::trigger(
        72,
        "memory_retrieval_enrichment",
        "retrieval_enrichment_compatibility_bd",
    ),
    SchemaInvariant::trigger(72, "memory_retrieval_enrichment", "memories_au"),
];
