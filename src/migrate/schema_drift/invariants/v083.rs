use super::SchemaInvariant;

pub(in crate::migrate) const V083_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::column(
        83,
        "retrieval_enrichment_budget",
        "memories",
        "search_context_enrichment_state",
    ),
    SchemaInvariant::index(
        83,
        "retrieval_enrichment_budget",
        "idx_memories_retrieval_enrichment_due",
    ),
    SchemaInvariant::trigger(83, "retrieval_enrichment_budget", "memories_au"),
];
