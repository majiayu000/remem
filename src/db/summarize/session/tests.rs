use anyhow::Result;
use rusqlite::{params, Connection};

use super::{finalize_summarize, upsert_session};

fn setup_summary_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE session_summaries (
            id INTEGER PRIMARY KEY,
            memory_session_id TEXT NOT NULL,
            project TEXT,
            request TEXT,
            completed TEXT,
            decisions TEXT,
            learned TEXT,
            next_steps TEXT,
            preferences TEXT,
            prompt_number INTEGER,
            created_at TEXT,
            created_at_epoch INTEGER,
            discovery_tokens INTEGER DEFAULT 0
        );
        CREATE TABLE summarize_cooldown (
            project TEXT PRIMARY KEY,
            last_summarize_epoch INTEGER NOT NULL,
            last_message_hash TEXT
        );
        CREATE TABLE sdk_sessions (
            id INTEGER PRIMARY KEY,
            content_session_id TEXT UNIQUE NOT NULL,
            memory_session_id TEXT NOT NULL,
            project TEXT,
            user_prompt TEXT,
            started_at TEXT,
            started_at_epoch INTEGER,
            status TEXT DEFAULT 'active',
            prompt_counter INTEGER DEFAULT 1
        );",
    )?;
    Ok(())
}

#[test]
fn finalize_summarize_replaces_in_single_commit() -> Result<()> {
    let mut conn = Connection::open_in_memory()?;
    setup_summary_schema(&conn)?;
    conn.execute(
        "INSERT INTO session_summaries (memory_session_id, project, request, created_at, created_at_epoch, discovery_tokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params!["mem-1", "proj", "old", "2026-01-01T00:00:00Z", 1_i64, 10_i64],
    )?;

    let deleted = finalize_summarize(
        &mut conn,
        "mem-1",
        "proj",
        "hash-1",
        Some("new"),
        Some("done"),
        Some("decision"),
        Some("learned"),
        Some("next"),
        Some("pref"),
        None,
        99,
    )?;
    assert_eq!(deleted, 1);

    let request: String = conn.query_row(
        "SELECT request FROM session_summaries WHERE memory_session_id = ?1 AND project = ?2",
        params!["mem-1", "proj"],
        |row| row.get(0),
    )?;
    assert_eq!(request, "new");

    let hash: String = conn.query_row(
        "SELECT last_message_hash FROM summarize_cooldown WHERE project = ?1",
        params!["proj"],
        |row| row.get(0),
    )?;
    assert_eq!(hash, "hash-1");
    Ok(())
}

#[test]
fn upsert_session_reuses_memory_session_id_and_increments_counter() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_summary_schema(&conn)?;

    let first = upsert_session(&conn, "content-session-abcdefghi", "proj", Some("hello"))?;
    let second = upsert_session(
        &conn,
        "content-session-abcdefghi",
        "proj",
        Some("hello again"),
    )?;

    assert_eq!(first, second);
    assert!(first.starts_with("mem-"));

    let prompt_counter: i64 = conn.query_row(
        "SELECT prompt_counter FROM sdk_sessions WHERE content_session_id = ?1",
        params!["content-session-abcdefghi"],
        |row| row.get(0),
    )?;
    assert_eq!(prompt_counter, 2);
    Ok(())
}
