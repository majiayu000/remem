use super::SchemaInvariant;

pub(in crate::migrate) const V089_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::column(
        89,
        "supplemental_local_copy_receipt",
        "memory_activation_requests",
        "local_copy_status",
    ),
    SchemaInvariant::column(
        89,
        "supplemental_local_copy_receipt",
        "memory_activation_requests",
        "local_copy_path",
    ),
    SchemaInvariant::column(
        89,
        "supplemental_local_copy_receipt",
        "memory_activation_requests",
        "local_copy_saved_at",
    ),
    SchemaInvariant::column(
        89,
        "supplemental_local_copy_receipt",
        "memory_activation_requests",
        "local_copy_sha256",
    ),
    SchemaInvariant::trigger(
        89,
        "supplemental_local_copy_receipt",
        "memory_activation_requests_local_copy_receipt_insert",
    ),
];
