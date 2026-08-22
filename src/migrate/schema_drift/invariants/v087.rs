use super::SchemaInvariant;

pub(in crate::migrate) const V087_SCHEMA_INVARIANTS: &[SchemaInvariant] =
    &[SchemaInvariant::column(
        87,
        "activation_result_trust",
        "memory_activation_requests",
        "result_source_trust_class",
    )];
