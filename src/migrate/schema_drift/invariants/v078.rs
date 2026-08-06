use super::SchemaInvariant;

pub(in crate::migrate) const V078_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::column(
        78,
        "event_capture_projection",
        "events",
        "captured_event_id",
    ),
    SchemaInvariant::index(78, "event_capture_projection", "idx_events_captured_event"),
];
