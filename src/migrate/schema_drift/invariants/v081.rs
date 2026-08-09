use super::SchemaInvariant;

pub(in crate::migrate) const V081_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::table(81, "context_bundle_audits", "context_bundle_audits"),
    SchemaInvariant::index(
        81,
        "context_bundle_audits",
        "idx_context_bundle_audits_created",
    ),
    SchemaInvariant::index(
        81,
        "context_bundle_audits",
        "idx_context_bundle_audits_plan",
    ),
    SchemaInvariant::trigger(
        81,
        "context_bundle_audits",
        "context_bundle_audits_require_items",
    ),
    SchemaInvariant::trigger(
        81,
        "context_bundle_audits",
        "context_bundle_audits_no_duplicate_insert",
    ),
    SchemaInvariant::trigger(
        81,
        "context_bundle_audits",
        "context_bundle_audits_immutable_update",
    ),
];
