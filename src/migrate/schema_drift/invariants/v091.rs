use super::SchemaInvariant;

pub(in crate::migrate) const V091_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::column(91, "raw_session_host", "raw_session_identities", "host"),
    SchemaInvariant::index(
        91,
        "raw_session_host",
        "idx_raw_session_identities_host_canonical",
    ),
];
