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
