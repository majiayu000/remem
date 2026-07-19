use super::SchemaInvariant;

macro_rules! identity_column {
    ($column:literal) => {
        SchemaInvariant::column(
            71,
            "raw_session_identity",
            "raw_session_identities",
            $column,
        )
    };
}

macro_rules! claim_column {
    ($column:literal) => {
        SchemaInvariant::column(
            71,
            "raw_session_identity",
            "raw_session_identity_claims",
            $column,
        )
    };
}

pub(in crate::migrate) const V071_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::table(71, "raw_session_identity", "raw_session_identities"),
    SchemaInvariant::table(71, "raw_session_identity", "raw_session_identity_claims"),
    identity_column!("id"),
    identity_column!("source_root"),
    identity_column!("transcript_path"),
    identity_column!("fallback_session_id"),
    identity_column!("canonical_session_id"),
    identity_column!("project"),
    identity_column!("legacy_project"),
    identity_column!("status"),
    identity_column!("conflict_reason"),
    identity_column!("contract_version"),
    identity_column!("event_index_status"),
    identity_column!("observed_mtime_ns"),
    identity_column!("observed_size_bytes"),
    identity_column!("first_event_epoch"),
    identity_column!("last_event_epoch"),
    identity_column!("missing_event_time_count"),
    identity_column!("first_seen_at_epoch"),
    identity_column!("last_seen_at_epoch"),
    claim_column!("id"),
    claim_column!("transcript_identity_id"),
    claim_column!("claimed_session_id"),
    claim_column!("identity_source"),
    claim_column!("first_seen_at_epoch"),
    claim_column!("last_seen_at_epoch"),
    SchemaInvariant::column(
        71,
        "raw_session_identity",
        "raw_messages",
        "event_time_source",
    ),
    SchemaInvariant::column(
        71,
        "raw_session_identity",
        "raw_messages",
        "transcript_identity_id",
    ),
    SchemaInvariant::column(
        71,
        "raw_session_identity",
        "raw_messages",
        "transcript_record_ordinal",
    ),
    SchemaInvariant::index(
        71,
        "raw_session_identity",
        "idx_raw_session_identities_fallback",
    ),
    SchemaInvariant::index(
        71,
        "raw_session_identity",
        "idx_raw_session_identities_canonical",
    ),
    SchemaInvariant::index(
        71,
        "raw_session_identity",
        "idx_raw_session_identity_claims_session",
    ),
    SchemaInvariant::index(
        71,
        "raw_session_identity",
        "idx_raw_messages_transcript_occurrence",
    ),
    SchemaInvariant::index(
        71,
        "raw_session_identity",
        "idx_raw_messages_non_transcript_content",
    ),
    SchemaInvariant::index(
        71,
        "raw_session_identity",
        "idx_raw_messages_transcript_time",
    ),
];
