use super::SchemaInvariant;

pub(in crate::migrate) const V067_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::table(67, "capture_git_evidence", "captured_event_commits"),
    SchemaInvariant::column(
        67,
        "capture_git_evidence",
        "git_commit_sessions",
        "session_row_id",
    ),
    SchemaInvariant::index(
        67,
        "capture_git_evidence",
        "idx_captured_event_commits_event",
    ),
    SchemaInvariant::index(
        67,
        "capture_git_evidence",
        "idx_git_commit_sessions_commit_session_row",
    ),
    SchemaInvariant::index(
        67,
        "capture_git_evidence",
        "idx_git_commit_sessions_commit_legacy_session",
    ),
    SchemaInvariant::index(
        67,
        "capture_git_evidence",
        "idx_git_commit_sessions_session_row",
    ),
];
