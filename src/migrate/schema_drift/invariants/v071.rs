use super::SchemaInvariant;

pub(in crate::migrate) const V071_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::table(71, "raw_session_identity", "raw_session_identities"),
    SchemaInvariant::table(71, "raw_session_identity", "raw_session_identity_claims"),
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
