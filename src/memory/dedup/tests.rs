use anyhow::Result;
use rusqlite::{params, Connection};

use super::{check_duplicate, find_hash_duplicates, mark_duplicate_accessed};

fn setup_dedup_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE observations (
            id INTEGER PRIMARY KEY,
            memory_session_id TEXT NOT NULL,
            project TEXT,
            type TEXT NOT NULL,
            title TEXT,
            subtitle TEXT,
            narrative TEXT,
            facts TEXT,
            concepts TEXT,
            files_read TEXT,
            files_modified TEXT,
            prompt_number INTEGER,
            created_at TEXT,
            created_at_epoch INTEGER,
            discovery_tokens INTEGER DEFAULT 0,
            status TEXT DEFAULT 'active',
            last_accessed_epoch INTEGER
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
        )",
    )?;
    Ok(())
}

fn insert_observation(conn: &Connection, project: &str, narrative: &str) -> Result<i64> {
    let now = chrono::Utc::now();
    conn.execute(
        "INSERT INTO observations \
         (memory_session_id, project, type, title, narrative, created_at, created_at_epoch, discovery_tokens, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            "mem-test",
            project,
            "bugfix",
            "Auth fix",
            narrative,
            now.to_rfc3339(),
            now.timestamp(),
            100,
            "active"
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

#[test]
fn test_hash_dedup_finds_exact_match() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_dedup_schema(&conn)?;

    let narrative = "Fixed authentication bug in login flow";
    insert_observation(&conn, "test-project", narrative)?;

    let content_hash = format!("{:x}", crate::db::deterministic_hash(narrative.as_bytes()));
    let dups = find_hash_duplicates(&conn, "test-project", &content_hash, 900)?;

    assert_eq!(dups.len(), 1);
    Ok(())
}

#[test]
fn mark_duplicate_accessed_updates_timestamp() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_dedup_schema(&conn)?;

    let id = insert_observation(&conn, "test-project", "same narrative")?;
    mark_duplicate_accessed(&conn, &[id])?;

    let last_accessed: Option<i64> = conn.query_row(
        "SELECT last_accessed_epoch FROM observations WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )?;
    assert!(last_accessed.is_some());
    Ok(())
}

#[test]
fn check_duplicate_returns_first_hash_duplicate() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_dedup_schema(&conn)?;

    let first = insert_observation(&conn, "test-project", "same narrative")?;
    let second = insert_observation(&conn, "test-project", "same narrative")?;

    let duplicate_id = check_duplicate(&conn, "test-project", "same narrative", None)?;

    assert_eq!(duplicate_id, Some(first));
    let last_accessed: Vec<Option<i64>> = [first, second]
        .iter()
        .map(|id| {
            conn.query_row(
                "SELECT last_accessed_epoch FROM observations WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert!(last_accessed.iter().all(|value| value.is_some()));
    Ok(())
}
