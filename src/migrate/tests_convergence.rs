use anyhow::Result;
use rusqlite::Connection;

use super::run_migrations;

/// The minimal v10 schema used to simulate an old database for upgrade tests.
/// Includes FTS virtual tables and triggers (present in all real v10+ databases).
const OLD_V10_SCHEMA: &str = "\
    CREATE TABLE sdk_sessions (id INTEGER PRIMARY KEY, content_session_id TEXT UNIQUE NOT NULL, memory_session_id TEXT NOT NULL, project TEXT, user_prompt TEXT, started_at TEXT, started_at_epoch INTEGER, status TEXT DEFAULT 'active', prompt_counter INTEGER DEFAULT 1);
    CREATE TABLE observations (id INTEGER PRIMARY KEY, memory_session_id TEXT NOT NULL, project TEXT, type TEXT NOT NULL, title TEXT, subtitle TEXT, narrative TEXT, facts TEXT, concepts TEXT, files_read TEXT, files_modified TEXT, prompt_number INTEGER, created_at TEXT, created_at_epoch INTEGER);
    CREATE TABLE session_summaries (id INTEGER PRIMARY KEY, memory_session_id TEXT NOT NULL, project TEXT, request TEXT, completed TEXT, decisions TEXT, learned TEXT, next_steps TEXT, preferences TEXT, prompt_number INTEGER, created_at TEXT, created_at_epoch INTEGER);
    CREATE TABLE pending_observations (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, project TEXT NOT NULL, tool_name TEXT NOT NULL, tool_input TEXT, tool_response TEXT, cwd TEXT, created_at_epoch INTEGER NOT NULL, lease_owner TEXT, lease_expires_epoch INTEGER);
    CREATE TABLE memories (id INTEGER PRIMARY KEY, session_id TEXT, project TEXT NOT NULL, topic_key TEXT, title TEXT NOT NULL, content TEXT NOT NULL, memory_type TEXT NOT NULL, files TEXT, created_at_epoch INTEGER NOT NULL, updated_at_epoch INTEGER NOT NULL, status TEXT NOT NULL DEFAULT 'active');
    CREATE TABLE events (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, project TEXT NOT NULL, event_type TEXT NOT NULL, summary TEXT NOT NULL, detail TEXT, files TEXT, exit_code INTEGER, created_at_epoch INTEGER NOT NULL);
    CREATE TABLE summarize_cooldown (project TEXT PRIMARY KEY, last_summarize_epoch INTEGER NOT NULL, last_message_hash TEXT);
    CREATE TABLE summarize_locks (project TEXT PRIMARY KEY, lock_epoch INTEGER NOT NULL);
    CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(title, content, content='memories', content_rowid='id', tokenize='trigram');
    CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN INSERT INTO memories_fts(rowid, title, content) VALUES (new.id, new.title, new.content); END;
    CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN INSERT INTO memories_fts(memories_fts, rowid, title, content) VALUES ('delete', old.id, old.title, old.content); END;
    CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN INSERT INTO memories_fts(memories_fts, rowid, title, content) VALUES ('delete', old.id, old.title, old.content); INSERT INTO memories_fts(rowid, title, content) VALUES (new.id, new.title, new.content); END;";

/// Tables whose columns must converge between fresh and upgraded databases.
/// FTS virtual tables and internal SQLite tables are excluded.
const CONVERGENCE_TABLES: &[&str] = &[
    "memories",
    "observations",
    "session_summaries",
    "pending_observations",
    "sdk_sessions",
    "events",
    "entities",
    "memory_entities",
    "memory_lessons",
    "ai_usage_events",
    "jobs",
    "workstreams",
    "workstream_sessions",
    "git_commits",
    "git_commit_sessions",
    "memory_state_keys",
    "topic_segments",
];

fn make_upgraded_v10_db() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA user_version = 10;")?;
    conn.execute_batch(OLD_V10_SCHEMA)?;
    run_migrations(&conn)?;
    Ok(conn)
}

fn make_fresh_db() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;
    Ok(conn)
}

fn normalize_sql_whitespace(sql: &str) -> String {
    sql.to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_identifier(token: &str) -> &str {
    token.trim_matches(|c| matches!(c, '"' | '\'' | '`' | '[' | ']' | '('))
}

fn defines_memories_table(sql: &str) -> bool {
    let normalized = normalize_sql_whitespace(sql);
    let tokens: Vec<&str> = normalized.split_whitespace().collect();

    tokens.iter().enumerate().any(|(index, token)| {
        if *token != "create" || tokens.get(index + 1) != Some(&"table") {
            return false;
        }

        let table_index = if tokens.get(index + 2..index + 5) == Some(&["if", "not", "exists"]) {
            index + 5
        } else {
            index + 2
        };

        tokens
            .get(table_index)
            .is_some_and(|name| normalize_identifier(name) == "memories")
    })
}

#[test]
fn memories_table_ddl_detection_collapses_whitespace() {
    assert!(defines_memories_table(
        "CREATE TABLE IF NOT EXISTS\n    memories (id INTEGER PRIMARY KEY)"
    ));
}

#[test]
fn memories_table_ddl_detection_requires_exact_table_name() {
    assert!(!defines_memories_table(
        "CREATE TABLE IF NOT EXISTS memories_backup (id INTEGER PRIMARY KEY)"
    ));
    assert!(!defines_memories_table(
        "CREATE TABLE IF NOT EXISTS memories_fts (title, content)"
    ));
}

#[test]
fn migrations_define_memories_table_once() -> Result<()> {
    let migrations_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("migrations");
    let mut ddl_sources = Vec::new();

    for entry in std::fs::read_dir(&migrations_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("sql") {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        assert!(
            !file_name.starts_with("schema_"),
            "secondary schema migration source must not exist: {file_name}"
        );

        let sql = std::fs::read_to_string(&path)?;
        if defines_memories_table(&sql) {
            ddl_sources.push(file_name);
        }
    }

    ddl_sources.sort();
    assert_eq!(
        ddl_sources,
        vec!["v001_baseline.sql".to_string()],
        "exactly one canonical memories table DDL source is allowed"
    );
    Ok(())
}

/// Return sorted column names for a table via PRAGMA table_info.
fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
    let mut cols: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();
    cols.sort();
    Ok(cols)
}

/// Return sorted index names for a table.
fn table_indexes(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='index' AND tbl_name=?1 \
         AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let indexes = stmt
        .query_map([table], |row| row.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(indexes)
}

/// Upgraded v10 DB must have the same columns as a fresh DB for all core tables.
/// This single test catches ALL future column divergence across ALL tables.
#[test]
fn columns_match_after_upgrade() -> Result<()> {
    let fresh = make_fresh_db()?;
    let upgraded = make_upgraded_v10_db()?;

    let mut mismatches = Vec::new();
    for table in CONVERGENCE_TABLES {
        let fresh_cols = table_columns(&fresh, table)?;
        let upgraded_cols = table_columns(&upgraded, table)?;

        if fresh_cols.is_empty() && upgraded_cols.is_empty() {
            continue;
        }

        let missing_in_upgraded: Vec<_> = fresh_cols
            .iter()
            .filter(|c| !upgraded_cols.contains(c))
            .collect();
        let extra_in_upgraded: Vec<_> = upgraded_cols
            .iter()
            .filter(|c| !fresh_cols.contains(c))
            .collect();

        if !missing_in_upgraded.is_empty() {
            mismatches.push(format!(
                "{}: columns in fresh but missing after upgrade: {:?}",
                table, missing_in_upgraded
            ));
        }
        if !extra_in_upgraded.is_empty() {
            mismatches.push(format!(
                "{}: extra columns after upgrade not in fresh: {:?}",
                table, extra_in_upgraded
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "schema divergence between fresh and upgraded DB:\n  {}",
        mismatches.join("\n  ")
    );
    Ok(())
}

/// Upgraded v10 DB must have the same indexes as a fresh DB for all core tables.
#[test]
fn indexes_match_after_upgrade() -> Result<()> {
    let fresh = make_fresh_db()?;
    let upgraded = make_upgraded_v10_db()?;

    let mut mismatches = Vec::new();
    for table in CONVERGENCE_TABLES {
        let fresh_idx = table_indexes(&fresh, table)?;
        let upgraded_idx = table_indexes(&upgraded, table)?;

        let missing: Vec<_> = fresh_idx
            .iter()
            .filter(|i| !upgraded_idx.contains(i))
            .collect();
        if !missing.is_empty() {
            mismatches.push(format!(
                "{}: indexes in fresh but missing after upgrade: {:?}",
                table, missing
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "index divergence between fresh and upgraded DB:\n  {}",
        mismatches.join("\n  ")
    );
    Ok(())
}

/// Verify the actual SQL queries used in search/read work against an upgraded DB.
/// This catches columns referenced in code but missing from both baseline and backfill.
#[test]
fn real_queries_work_on_upgraded_db() -> Result<()> {
    use crate::memory::{memory_current_filter_sql, MEMORY_COLS};

    let conn = make_upgraded_v10_db()?;
    let current_filter = memory_current_filter_sql("m.status", "m.expires_at_epoch", false);

    // FTS search query (from memory_search/fts.rs)
    let fts_query = format!(
        "SELECT m.{} FROM memories m \
         JOIN memories_fts ON memories_fts.rowid = m.id \
         WHERE memories_fts MATCH 'test' \
           AND {} \
         ORDER BY bm25(memories_fts, 10.0, 1.0, 3.0) LIMIT 10",
        MEMORY_COLS.replace(", ", ", m."),
        current_filter
    );
    assert!(
        conn.prepare(&fts_query).is_ok(),
        "FTS search query must work on upgraded DB"
    );

    // LIKE fallback query (from memory_search/like.rs)
    let current_filter = memory_current_filter_sql("m.status", "m.expires_at_epoch", false);
    let like_query = format!(
        "SELECT m.{} FROM memories m \
         WHERE m.content LIKE '%test%' \
           AND {} \
         ORDER BY m.updated_at_epoch DESC LIMIT 10",
        MEMORY_COLS.replace(", ", ", m."),
        current_filter
    );
    assert!(
        conn.prepare(&like_query).is_ok(),
        "LIKE fallback query must work on upgraded DB"
    );

    // Scope-aware read query (from memory/store/read.rs)
    let current_filter = memory_current_filter_sql("status", "expires_at_epoch", false);
    let scope_query = format!(
        "SELECT {} FROM memories \
         WHERE (project = 'test' OR scope = 'global') AND {} \
         ORDER BY updated_at_epoch DESC LIMIT 10",
        MEMORY_COLS, current_filter
    );
    assert!(
        conn.prepare(&scope_query).is_ok(),
        "scope-aware read query must work on upgraded DB"
    );

    Ok(())
}

/// #244: PRAGMA user_version must stay consistent with the highest version
/// recorded in _schema_migrations. Both a fresh DB and an upgraded legacy DB
/// must report user_version == OLD_BASELINE_VERSION - 1 + max_applied.
#[test]
fn user_version_tracks_max_applied_migration() -> Result<()> {
    let expected = super::types::OLD_BASELINE_VERSION - 1 + super::latest_schema_version();

    for conn in [make_fresh_db()?, make_upgraded_v10_db()?] {
        let max_applied = super::state::applied_versions(&conn)?
            .into_iter()
            .max()
            .expect("at least one migration applied");
        let user_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

        assert_eq!(
            max_applied,
            super::latest_schema_version(),
            "all migrations must be recorded in _schema_migrations"
        );
        assert_eq!(
            user_version, expected,
            "user_version must equal OLD_BASELINE_VERSION-1 + max applied version"
        );
    }
    Ok(())
}

/// #244: all known migrations must end up recorded in _schema_migrations after
/// a legacy upgrade, so PRAGMA user_version (derived from the max applied
/// version) cannot drift ahead of what was actually applied.
#[test]
fn legacy_upgrade_records_every_migration() -> Result<()> {
    let conn = make_upgraded_v10_db()?;
    let applied = super::state::applied_versions(&conn)?;
    for migration in super::types::MIGRATIONS {
        assert!(
            applied.contains(&migration.version),
            "migration v{} must be recorded after upgrade: {applied:?}",
            migration.version
        );
    }
    Ok(())
}

/// #244: foreign_keys must be ON for the runtime connection so ON DELETE
/// CASCADE / SET NULL actually fire.
#[test]
fn open_db_enables_foreign_keys() -> Result<()> {
    let _guard = crate::db::test_support::ScopedTestDataDir::new("fk-pragma");
    let conn = crate::db::open_db()?;
    let fk_on: i64 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    assert_eq!(fk_on, 1, "open_db must enable PRAGMA foreign_keys");
    Ok(())
}
