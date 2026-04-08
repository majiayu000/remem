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
    "ai_usage_events",
    "jobs",
    "workstreams",
    "workstream_sessions",
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
    use crate::memory::types::MEMORY_COLS;

    let conn = make_upgraded_v10_db()?;

    // FTS search query (from memory_search/fts.rs)
    let fts_query = format!(
        "SELECT m.{} FROM memories m \
         JOIN memories_fts ON memories_fts.rowid = m.id \
         WHERE memories_fts MATCH 'test' \
         ORDER BY bm25(memories_fts, 10.0, 1.0) LIMIT 10",
        MEMORY_COLS.replace(", ", ", m.")
    );
    assert!(
        conn.prepare(&fts_query).is_ok(),
        "FTS search query must work on upgraded DB"
    );

    // LIKE fallback query (from memory_search/like.rs)
    let like_query = format!(
        "SELECT m.{} FROM memories m \
         WHERE m.content LIKE '%test%' \
         ORDER BY m.updated_at_epoch DESC LIMIT 10",
        MEMORY_COLS.replace(", ", ", m.")
    );
    assert!(
        conn.prepare(&like_query).is_ok(),
        "LIKE fallback query must work on upgraded DB"
    );

    // Scope-aware read query (from memory/store/read.rs)
    let scope_query = format!(
        "SELECT {} FROM memories \
         WHERE (project = 'test' OR scope = 'global') AND status = 'active' \
         ORDER BY updated_at_epoch DESC LIMIT 10",
        MEMORY_COLS
    );
    assert!(
        conn.prepare(&scope_query).is_ok(),
        "scope-aware read query must work on upgraded DB"
    );

    Ok(())
}
