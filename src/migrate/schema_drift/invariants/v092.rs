use super::SchemaInvariant;

pub(in crate::migrate) const V092_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::column(
        92,
        "session_intent_display",
        "session_summaries",
        "session_intent",
    ),
    SchemaInvariant::column(
        92,
        "session_intent_display",
        "session_summaries",
        "session_topic",
    ),
    SchemaInvariant::column(
        92,
        "session_intent_display",
        "session_summaries",
        "session_intent_source",
    ),
    SchemaInvariant::column(
        92,
        "session_intent_display",
        "session_summaries",
        "session_intent_updated_at_epoch",
    ),
    SchemaInvariant::column(
        92,
        "session_intent_display",
        "workstreams",
        "session_intent",
    ),
    SchemaInvariant::column(92, "session_intent_display", "workstreams", "session_topic"),
    SchemaInvariant::column(
        92,
        "session_intent_display",
        "workstreams",
        "session_intent_source",
    ),
    SchemaInvariant::column(
        92,
        "session_intent_display",
        "workstreams",
        "session_intent_updated_at_epoch",
    ),
];
