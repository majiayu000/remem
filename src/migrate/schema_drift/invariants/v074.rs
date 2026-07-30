use super::SchemaInvariant;

const VERSION: i64 = 74;
const MIGRATION: &str = "git_commit_staleness_index";

pub(in crate::migrate) const V074_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::table(VERSION, MIGRATION, "git_commit_files"),
    SchemaInvariant::index(VERSION, MIGRATION, "idx_git_commits_project_commit_epoch"),
    SchemaInvariant::trigger(
        VERSION,
        MIGRATION,
        "git_commits_validate_changed_files_insert",
    ),
    SchemaInvariant::trigger(
        VERSION,
        MIGRATION,
        "git_commits_validate_changed_files_update",
    ),
    SchemaInvariant::trigger(VERSION, MIGRATION, "git_commits_sync_files_insert"),
    SchemaInvariant::trigger(VERSION, MIGRATION, "git_commits_sync_files_update"),
    SchemaInvariant::trigger(VERSION, MIGRATION, "git_commits_sync_files_delete"),
];
