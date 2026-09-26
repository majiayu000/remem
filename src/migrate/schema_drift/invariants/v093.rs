use super::SchemaInvariant;

pub(in crate::migrate) const V093_SCHEMA_INVARIANTS: &[SchemaInvariant] =
    &[SchemaInvariant::column(
        93,
        "raw_session_mode_version",
        "raw_session_identities",
        "session_mode_version",
    )];
