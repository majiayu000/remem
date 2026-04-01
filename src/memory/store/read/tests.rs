use super::*;
use crate::memory::tests_helper::setup_memory_schema;
use crate::memory::{insert_memory, search_memories_fts, search_memories_like};
use rusqlite::Connection;

#[test]
fn test_memory_insert_and_query() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let id = insert_memory(
        &conn,
        Some("session-1"),
        "test/proj",
        None,
        "FTS5 supports CJK",
        "Switched from unicode61 to trigram tokenizer for Chinese text search.",
        "decision",
        Some(r#"["src/db.rs"]"#),
    )
    .unwrap();
    assert!(id > 0);

    let memories = get_recent_memories(&conn, "test/proj", 10).unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].title, "FTS5 supports CJK");
}

#[test]
fn test_memory_fts_search() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    insert_memory(
        &conn,
        Some("s1"),
        "proj",
        None,
        "FTS5 trigram tokenizer 支持 CJK",
        "Switched to trigram for Chinese search support.",
        "decision",
        None,
    )
    .unwrap();
    insert_memory(
        &conn,
        Some("s1"),
        "proj",
        None,
        "Auth middleware rewrite",
        "Rewrote auth middleware for compliance.",
        "architecture",
        None,
    )
    .unwrap();

    let results = search_memories_fts(&conn, "trigram", Some("proj"), None, 10, 0).unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].title.contains("trigram"));
}

#[test]
fn test_memory_like_fallback() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    insert_memory(
        &conn,
        Some("s1"),
        "proj",
        None,
        "DB schema migration",
        "Updated schema from v7 to v8.",
        "decision",
        None,
    )
    .unwrap();

    let results = search_memories_like(&conn, &["DB"], Some("proj"), None, 10, 0).unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].title.contains("DB"));
}

#[test]
fn test_memory_type_filter() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    insert_memory(
        &conn,
        Some("s1"),
        "proj",
        None,
        "Bug: unicode61 fails CJK",
        "Root cause: unicode61 tokenizer doesn't segment Chinese.",
        "bugfix",
        None,
    )
    .unwrap();
    insert_memory(
        &conn,
        Some("s1"),
        "proj",
        None,
        "Use trigram tokenizer",
        "Decided to use trigram for CJK support.",
        "decision",
        None,
    )
    .unwrap();

    assert_eq!(get_memories_by_type(&conn, "proj", "bugfix", 10).unwrap().len(), 1);
    assert_eq!(
        get_memories_by_type(&conn, "proj", "decision", 10)
            .unwrap()
            .len(),
        1
    );
}
