use super::SchemaInvariant;

macro_rules! poisoning_column {
    ($column:literal) => {
        SchemaInvariant::column(
            73,
            "session_summary_poisoning",
            "session_summaries",
            $column,
        )
    };
}

pub(in crate::migrate) const V073_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    poisoning_column!("poisoning_status"),
    poisoning_column!("quarantine_stage"),
    poisoning_column!("quarantine_field"),
    poisoning_column!("quarantine_event_id"),
    poisoning_column!("quarantine_pattern_id"),
    poisoning_column!("quarantine_pattern_version"),
    poisoning_column!("acknowledged_pattern_id"),
    poisoning_column!("acknowledged_pattern_version"),
    poisoning_column!("acknowledged_at_epoch"),
    poisoning_column!("poisoning_block_count"),
    poisoning_column!("poisoning_last_blocked_at_epoch"),
    SchemaInvariant::index(
        73,
        "session_summary_poisoning",
        "idx_session_summaries_poisoning",
    ),
];
