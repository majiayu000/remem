use super::SchemaInvariant;

pub(in crate::migrate) const V084_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::table(84, "session_observatory", "session_turns"),
    SchemaInvariant::table(84, "session_observatory", "session_turn_actions"),
    SchemaInvariant::index(84, "session_observatory", "idx_session_turns_recent"),
    SchemaInvariant::index(
        84,
        "session_observatory",
        "idx_session_turns_project_recent",
    ),
    SchemaInvariant::index(84, "session_observatory", "idx_session_turn_actions_event"),
];
