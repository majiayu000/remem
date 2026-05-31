use super::*;
use crate::memory::get_recent_memories;
use crate::memory::tests_helper::setup_memory_schema;
use rusqlite::Connection;

#[test]
fn test_topic_key_upsert() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let id1 = insert_memory(
        &conn,
        Some("s1"),
        "test/proj",
        Some("fts5-search-strategy"),
        "FTS5 trigram v1",
        "Initial implementation using trigram.",
        "decision",
        None,
    )
    .unwrap();
    let id2 = insert_memory(
        &conn,
        Some("s2"),
        "test/proj",
        Some("fts5-search-strategy"),
        "FTS5 trigram v2",
        "Added LIKE fallback for short tokens.",
        "decision",
        None,
    )
    .unwrap();

    assert_eq!(id1, id2);
    let memories = get_recent_memories(&conn, "test/proj", 10).unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].title, "FTS5 trigram v2");
}

#[test]
fn test_topic_key_upsert_reactivates_stale_row() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let id1 = insert_memory(
        &conn,
        Some("s1"),
        "test/proj",
        Some("deploy-runbook"),
        "Deploy runbook v1",
        "Old steps.",
        "procedure",
        None,
    )
    .unwrap();

    // Simulate the row aging out (e.g. via cleanup) into a non-active state.
    conn.execute(
        "UPDATE memories SET status = 'stale' WHERE id = ?1",
        params![id1],
    )
    .unwrap();

    // The same topic_key is upserted with fresh content.
    let id2 = insert_memory(
        &conn,
        Some("s2"),
        "test/proj",
        Some("deploy-runbook"),
        "Deploy runbook v2",
        "New steps after fix.",
        "procedure",
        None,
    )
    .unwrap();
    assert_eq!(id1, id2);

    // The row must be reactivated so the updated content is visible again.
    let status: String = conn
        .query_row(
            "SELECT status FROM memories WHERE id = ?1",
            params![id1],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "active");

    let memories = get_recent_memories(&conn, "test/proj", 10).unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].title, "Deploy runbook v2");
    assert_eq!(memories[0].text, "New steps after fix.");
}

#[test]
fn test_topic_key_upsert_reactivates_archived_row() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let id1 = insert_memory(
        &conn,
        Some("s1"),
        "test/proj",
        Some("api-token-rotation"),
        "Token rotation v1",
        "Original notes.",
        "decision",
        None,
    )
    .unwrap();

    conn.execute(
        "UPDATE memories SET status = 'archived' WHERE id = ?1",
        params![id1],
    )
    .unwrap();

    let id2 = insert_memory(
        &conn,
        Some("s2"),
        "test/proj",
        Some("api-token-rotation"),
        "Token rotation v2",
        "Rotation now automated.",
        "decision",
        None,
    )
    .unwrap();
    assert_eq!(id1, id2);

    let status: String = conn
        .query_row(
            "SELECT status FROM memories WHERE id = ?1",
            params![id1],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "active");

    let memories = get_recent_memories(&conn, "test/proj", 10).unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].title, "Token rotation v2");
}

#[test]
fn test_created_at_override() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let custom_epoch: i64 = 1_700_000_000;
    let id = insert_memory_full(
        &conn,
        Some("s1"),
        "proj",
        None,
        "Old event",
        "Something from the past",
        "discovery",
        None,
        None,
        "project",
        Some(custom_epoch),
    )
    .unwrap();

    let row: (i64, i64) = conn
        .query_row(
            "SELECT created_at_epoch, updated_at_epoch FROM memories WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(row.0, custom_epoch);
    assert_ne!(row.1, custom_epoch);
}

#[test]
fn test_created_at_default_when_no_override() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let before = chrono::Utc::now().timestamp();
    let id = insert_memory_full(
        &conn,
        Some("s1"),
        "proj",
        None,
        "Recent event",
        "Something now",
        "discovery",
        None,
        None,
        "project",
        None,
    )
    .unwrap();
    let after = chrono::Utc::now().timestamp();

    let created: i64 = conn
        .query_row(
            "SELECT created_at_epoch FROM memories WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(created >= before && created <= after);
}
