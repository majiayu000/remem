use anyhow::Result;
use rusqlite::Connection;

use super::state::applied_versions;
use super::{dry_run_pending, run_migrations, MIGRATIONS};
use crate::db::test_support::ScopedTestDataDir;

#[test]
fn baseline_creates_all_tables() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(MIGRATIONS[0].sql)?;

    let expected_tables = [
        "sdk_sessions",
        "observations",
        "session_summaries",
        "pending_observations",
        "memories",
        "events",
        "entities",
        "memory_entities",
        "summarize_cooldown",
        "summarize_locks",
        "ai_usage_events",
        "jobs",
        "workstreams",
        "workstream_sessions",
    ];
    for table in &expected_tables {
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |_| Ok(true),
            )
            .unwrap_or(false);
        assert!(exists, "table {} not created by baseline", table);
    }
    Ok(())
}

#[test]
fn migration_sql_has_no_nonconstant_alter_defaults() {
    for migration in MIGRATIONS {
        for line in migration.sql.lines() {
            let upper = line.trim().to_uppercase();
            assert!(
                !(upper.starts_with("ALTER TABLE") && upper.contains("DEFAULT (")),
                "v{:03}_{} has non-constant DEFAULT in ALTER TABLE: {}",
                migration.version,
                migration.name,
                line.trim()
            );
        }
    }
}

#[test]
fn full_migration_on_empty_db() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    let applied = applied_versions(&conn)?;
    assert_eq!(applied, vec![1]);

    let user_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    assert_eq!(user_version, 13);
    Ok(())
}

#[test]
fn transition_from_old_system_skips_baseline() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA user_version = 13;")?;
    conn.execute_batch(MIGRATIONS[0].sql)?;

    run_migrations(&conn)?;

    let applied = applied_versions(&conn)?;
    assert_eq!(applied, vec![1]);
    Ok(())
}

#[test]
fn auto_upgrades_old_schema_version() -> Result<()> {
    let _test_dir = ScopedTestDataDir::new("migrate-auto-upgrade");
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA user_version = 10;")?;
    // Simulate a v10 database with minimal tables
    conn.execute_batch(
        "CREATE TABLE sdk_sessions (id INTEGER PRIMARY KEY, content_session_id TEXT UNIQUE NOT NULL, memory_session_id TEXT NOT NULL, project TEXT, user_prompt TEXT, started_at TEXT, started_at_epoch INTEGER, status TEXT DEFAULT 'active', prompt_counter INTEGER DEFAULT 1);
         CREATE TABLE observations (id INTEGER PRIMARY KEY, memory_session_id TEXT NOT NULL, project TEXT, type TEXT NOT NULL, title TEXT, subtitle TEXT, narrative TEXT, facts TEXT, concepts TEXT, files_read TEXT, files_modified TEXT, prompt_number INTEGER, created_at TEXT, created_at_epoch INTEGER);
         CREATE TABLE session_summaries (id INTEGER PRIMARY KEY, memory_session_id TEXT NOT NULL, project TEXT, request TEXT, completed TEXT, decisions TEXT, learned TEXT, next_steps TEXT, preferences TEXT, prompt_number INTEGER, created_at TEXT, created_at_epoch INTEGER);
         CREATE TABLE pending_observations (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, project TEXT NOT NULL, tool_name TEXT NOT NULL, tool_input TEXT, tool_response TEXT, cwd TEXT, created_at_epoch INTEGER NOT NULL, lease_owner TEXT, lease_expires_epoch INTEGER);
         CREATE TABLE memories (id INTEGER PRIMARY KEY, session_id TEXT, project TEXT NOT NULL, topic_key TEXT, title TEXT NOT NULL, content TEXT NOT NULL, memory_type TEXT NOT NULL, files TEXT, created_at_epoch INTEGER NOT NULL, updated_at_epoch INTEGER NOT NULL, status TEXT NOT NULL DEFAULT 'active');
         CREATE TABLE events (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, project TEXT NOT NULL, event_type TEXT NOT NULL, summary TEXT NOT NULL, detail TEXT, files TEXT, exit_code INTEGER, created_at_epoch INTEGER NOT NULL);
         CREATE TABLE summarize_cooldown (project TEXT PRIMARY KEY, last_summarize_epoch INTEGER NOT NULL, last_message_hash TEXT);
         CREATE TABLE summarize_locks (project TEXT PRIMARY KEY, lock_epoch INTEGER NOT NULL);",
    )?;

    run_migrations(&conn)?;

    // Should have auto-upgraded and marked baseline as applied
    let applied = applied_versions(&conn)?;
    assert_eq!(applied, vec![1]);

    let user_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    assert_eq!(user_version, 13);

    // Verify missing columns were added
    let has_status: bool = conn
        .prepare("SELECT status FROM pending_observations LIMIT 0")
        .is_ok();
    assert!(has_status, "pending_observations.status should exist");

    // Verify missing tables were created
    let has_entities: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='entities'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    assert!(has_entities, "entities table should exist after backfill");
    Ok(())
}

#[test]
fn dry_run_pending_reports_no_pending_for_current_schema() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    run_migrations(&conn)?;

    let result = dry_run_pending(&conn)?;

    assert_eq!(result.pending_count, 0);
    assert!(result.error.is_none());
    Ok(())
}

#[test]
fn dry_run_pending_reports_pending_for_new_db() -> Result<()> {
    let conn = Connection::open_in_memory()?;

    let result = dry_run_pending(&conn)?;

    assert_eq!(result.current_version, 0);
    assert_eq!(result.pending_count, 1);
    assert!(result.error.is_none());
    Ok(())
}

/// Regression: a v13 DB with migration entries but missing `scope` column
/// must have `scope` added by backfill on next startup.
#[test]
fn backfill_runs_even_when_migration_entries_exist() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA user_version = 13;")?;
    create_v13_schema_without_scope(&conn)?;

    // Pre-populate _schema_migrations so transition thinks it already ran
    conn.execute_batch(
        "CREATE TABLE _schema_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at_epoch INTEGER NOT NULL);
         INSERT INTO _schema_migrations VALUES (1, 'baseline', 1700000000);",
    )?;

    run_migrations(&conn)?;

    // scope column must exist and be queryable
    let has_scope: bool = conn.prepare("SELECT scope FROM memories LIMIT 0").is_ok();
    assert!(has_scope, "memories.scope must exist after backfill");
    Ok(())
}

#[test]
fn backfill_fails_when_non_duplicate_alter_table_error_occurs() -> Result<()> {
    let _test_dir = ScopedTestDataDir::new("migrate-backfill-error");
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA user_version = 13;")?;
    create_v13_schema_without_scope(&conn)?;
    conn.execute_batch("DROP TABLE pending_observations;")?;
    conn.execute_batch(
        "CREATE TABLE _schema_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at_epoch INTEGER NOT NULL);
         INSERT INTO _schema_migrations VALUES (1, 'baseline', 1700000000);",
    )?;

    let error = run_migrations(&conn).expect_err("missing table should fail backfill");
    let message = error.to_string();
    assert!(message.contains("backfill pending_observations.updated_at_epoch failed"));
    Ok(())
}

#[test]
fn dry_run_pending_reports_backfill_error_for_broken_schema() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA user_version = 13;")?;
    create_v13_schema_without_scope(&conn)?;
    conn.execute_batch("DROP TABLE pending_observations;")?;
    conn.execute_batch(
        "CREATE TABLE _schema_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at_epoch INTEGER NOT NULL);
         INSERT INTO _schema_migrations VALUES (1, 'baseline', 1700000000);",
    )?;

    let result = dry_run_pending(&conn)?;
    assert_eq!(result.pending_count, 0);
    let error = result
        .error
        .expect("broken schema should surface in dry-run");
    assert!(error.contains("baseline backfill"));
    assert!(error.contains("backfill pending_observations.updated_at_epoch failed"));
    Ok(())
}

/// Regression: clone_schema used to skip ALL underscore-prefixed tables but not
/// their dependent indexes. A non-migration _-prefixed table with an explicit index
/// caused clone_schema to fail with "no such table" because the table DDL was
/// omitted while the index DDL was still executed.
#[test]
fn dry_run_clones_non_migration_underscore_table_with_dependent_index() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA user_version = 13;")?;
    create_v13_schema_without_scope(&conn)?;
    conn.execute_batch(
        "CREATE TABLE _schema_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at_epoch INTEGER NOT NULL);
         INSERT INTO _schema_migrations VALUES (1, 'baseline', 1700000000);",
    )?;
    // An app-owned underscore-prefixed table with an explicit index.
    // Old broad-skip code omitted the table but still executed the index DDL,
    // producing a "no such table" clone error.
    conn.execute_batch(
        "CREATE TABLE _app_cache (id INTEGER PRIMARY KEY, key TEXT NOT NULL);
         CREATE INDEX idx_app_cache_key ON _app_cache(key);",
    )?;

    let result = dry_run_pending(&conn)?;
    assert!(
        result.error.is_none(),
        "clone_schema must not fail for non-migration underscore tables with indexes: {:?}",
        result.error
    );
    Ok(())
}

/// Regression: clone_schema used SQL-prefix matching to identify _schema_migrations,
/// which is sensitive to quoting. Bracket-quoted DDL (`CREATE TABLE [_schema_migrations]`)
/// was not caught, allowing the internal table to bleed into the dry-run database.
/// Matching on the unquoted `name` column from sqlite_master is immune to quoting.
#[test]
fn dry_run_skips_schema_migrations_regardless_of_sql_quoting() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA user_version = 13;")?;
    create_v13_schema_without_scope(&conn)?;
    // Bracket-quoted form — sqlite_master retains the brackets in `sql` but
    // the `name` column is always the bare identifier.
    conn.execute_batch(
        "CREATE TABLE [_schema_migrations] (version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at_epoch INTEGER NOT NULL);
         INSERT INTO [_schema_migrations] VALUES (1, 'baseline', 1700000000);",
    )?;

    let result = dry_run_pending(&conn)?;
    assert!(
        result.error.is_none(),
        "dry_run must not fail with bracket-quoted _schema_migrations: {:?}",
        result.error
    );
    Ok(())
}

fn create_v13_schema_without_scope(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE memories (
            id INTEGER PRIMARY KEY, session_id TEXT, project TEXT NOT NULL,
            topic_key TEXT, title TEXT NOT NULL, content TEXT NOT NULL,
            memory_type TEXT NOT NULL, files TEXT,
            created_at_epoch INTEGER NOT NULL, updated_at_epoch INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'active', branch TEXT
        );
        CREATE TABLE sdk_sessions (id INTEGER PRIMARY KEY, content_session_id TEXT UNIQUE NOT NULL, memory_session_id TEXT NOT NULL, project TEXT, user_prompt TEXT, started_at TEXT, started_at_epoch INTEGER, status TEXT DEFAULT 'active', prompt_counter INTEGER DEFAULT 1);
        CREATE TABLE observations (id INTEGER PRIMARY KEY, memory_session_id TEXT NOT NULL, project TEXT, type TEXT NOT NULL, title TEXT, subtitle TEXT, narrative TEXT, facts TEXT, concepts TEXT, files_read TEXT, files_modified TEXT, prompt_number INTEGER, created_at TEXT, created_at_epoch INTEGER, discovery_tokens INTEGER DEFAULT 0, status TEXT DEFAULT 'active', last_accessed_epoch INTEGER, branch TEXT, commit_sha TEXT);
        CREATE TABLE session_summaries (id INTEGER PRIMARY KEY, memory_session_id TEXT NOT NULL, project TEXT, request TEXT, completed TEXT, decisions TEXT, learned TEXT, next_steps TEXT, preferences TEXT, prompt_number INTEGER, created_at TEXT, created_at_epoch INTEGER, discovery_tokens INTEGER DEFAULT 0);
        CREATE TABLE pending_observations (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, project TEXT NOT NULL, tool_name TEXT NOT NULL, tool_input TEXT, tool_response TEXT, cwd TEXT, created_at_epoch INTEGER NOT NULL, updated_at_epoch INTEGER NOT NULL DEFAULT 0, status TEXT NOT NULL DEFAULT 'pending', attempt_count INTEGER NOT NULL DEFAULT 0, next_retry_epoch INTEGER, last_error TEXT, lease_owner TEXT, lease_expires_epoch INTEGER);
        CREATE TABLE events (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, project TEXT NOT NULL, event_type TEXT NOT NULL, summary TEXT NOT NULL, detail TEXT, files TEXT, exit_code INTEGER, created_at_epoch INTEGER NOT NULL);
        CREATE TABLE summarize_cooldown (project TEXT PRIMARY KEY, last_summarize_epoch INTEGER NOT NULL, last_message_hash TEXT);
        CREATE TABLE summarize_locks (project TEXT PRIMARY KEY, lock_epoch INTEGER NOT NULL);",
    )?;
    Ok(())
}

/// Verify all columns in MEMORY_COLS are present after migrating from any starting state.
#[test]
fn memory_cols_all_present_after_migration() -> Result<()> {
    use crate::memory::types::MEMORY_COLS;

    let expected_cols: Vec<&str> = MEMORY_COLS.split(',').map(|s| s.trim()).collect();

    // Test from empty DB
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    let query = format!("SELECT {} FROM memories LIMIT 0", MEMORY_COLS);
    assert!(
        conn.prepare(&query).is_ok(),
        "all MEMORY_COLS must be queryable on fresh DB: {:?}",
        expected_cols
    );

    // Test from old v10 DB (no scope, no branch)
    let conn2 = Connection::open_in_memory()?;
    conn2.execute_batch("PRAGMA user_version = 10;")?;
    conn2.execute_batch(
        "CREATE TABLE sdk_sessions (id INTEGER PRIMARY KEY, content_session_id TEXT UNIQUE NOT NULL, memory_session_id TEXT NOT NULL, project TEXT, user_prompt TEXT, started_at TEXT, started_at_epoch INTEGER, status TEXT DEFAULT 'active', prompt_counter INTEGER DEFAULT 1);
         CREATE TABLE observations (id INTEGER PRIMARY KEY, memory_session_id TEXT NOT NULL, project TEXT, type TEXT NOT NULL, title TEXT, subtitle TEXT, narrative TEXT, facts TEXT, concepts TEXT, files_read TEXT, files_modified TEXT, prompt_number INTEGER, created_at TEXT, created_at_epoch INTEGER);
         CREATE TABLE session_summaries (id INTEGER PRIMARY KEY, memory_session_id TEXT NOT NULL, project TEXT, request TEXT, completed TEXT, decisions TEXT, learned TEXT, next_steps TEXT, preferences TEXT, prompt_number INTEGER, created_at TEXT, created_at_epoch INTEGER);
         CREATE TABLE pending_observations (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, project TEXT NOT NULL, tool_name TEXT NOT NULL, tool_input TEXT, tool_response TEXT, cwd TEXT, created_at_epoch INTEGER NOT NULL, lease_owner TEXT, lease_expires_epoch INTEGER);
         CREATE TABLE memories (id INTEGER PRIMARY KEY, session_id TEXT, project TEXT NOT NULL, topic_key TEXT, title TEXT NOT NULL, content TEXT NOT NULL, memory_type TEXT NOT NULL, files TEXT, created_at_epoch INTEGER NOT NULL, updated_at_epoch INTEGER NOT NULL, status TEXT NOT NULL DEFAULT 'active');
         CREATE TABLE events (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, project TEXT NOT NULL, event_type TEXT NOT NULL, summary TEXT NOT NULL, detail TEXT, files TEXT, exit_code INTEGER, created_at_epoch INTEGER NOT NULL);
         CREATE TABLE summarize_cooldown (project TEXT PRIMARY KEY, last_summarize_epoch INTEGER NOT NULL, last_message_hash TEXT);
         CREATE TABLE summarize_locks (project TEXT PRIMARY KEY, lock_epoch INTEGER NOT NULL);",
    )?;
    run_migrations(&conn2)?;

    let query2 = format!("SELECT {} FROM memories LIMIT 0", MEMORY_COLS);
    assert!(
        conn2.prepare(&query2).is_ok(),
        "all MEMORY_COLS must be queryable after v10 upgrade: {:?}",
        expected_cols
    );

    Ok(())
}
