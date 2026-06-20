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
fn mark_memories_accessed_updates_usage_columns_once_per_call() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let first = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        None,
        "Usage target",
        "Accessed through full-detail retrieval.",
        "decision",
        None,
    )
    .unwrap();
    let second = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        None,
        "Other target",
        "Also accessed through full-detail retrieval.",
        "decision",
        None,
    )
    .unwrap();

    mark_memories_accessed(&conn, &[first, second, first]).unwrap();
    mark_memories_accessed(&conn, &[first]).unwrap();

    let first_usage: (i64, Option<i64>) = conn
        .query_row(
            "SELECT access_count, last_accessed_epoch FROM memories WHERE id = ?1",
            [first],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let second_usage: (i64, Option<i64>) = conn
        .query_row(
            "SELECT access_count, last_accessed_epoch FROM memories WHERE id = ?1",
            [second],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    assert_eq!(first_usage.0, 2);
    assert!(first_usage.1.is_some());
    assert_eq!(second_usage.0, 1);
    assert!(second_usage.1.is_some());
}

#[test]
fn get_memories_by_ids_hides_policy_suppressed_rows_by_default() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let visible = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        None,
        "Visible target",
        "Visible direct-read result.",
        "decision",
        None,
    )
    .unwrap();
    let hidden = insert_memory(
        &conn,
        Some("s2"),
        "proj",
        None,
        "Hidden target",
        "Hidden direct-read result.",
        "decision",
        None,
    )
    .unwrap();
    crate::memory::suppression::create_suppression(
        &conn,
        &crate::memory::suppression::SuppressRequest {
            target: crate::memory::suppression::parse_target(&format!("memory:{hidden}")).unwrap(),
            reason: Some("not useful"),
            actor: Some("test"),
        },
    )
    .unwrap();

    let default = get_memories_by_ids(&conn, &[visible, hidden], Some("proj")).unwrap();
    let default_ids = default.iter().map(|memory| memory.id).collect::<Vec<_>>();
    assert_eq!(default_ids, vec![visible]);

    let audit =
        get_memories_by_ids_with_suppressed_policy(&conn, &[visible, hidden], Some("proj"), true)
            .unwrap();
    let audit_ids = audit.iter().map(|memory| memory.id).collect::<Vec<_>>();
    assert!(audit_ids.contains(&visible));
    assert!(audit_ids.contains(&hidden));
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

    assert_eq!(
        get_memories_by_type(&conn, "proj", "bugfix", 10)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        get_memories_by_type(&conn, "proj", "decision", 10)
            .unwrap()
            .len(),
        1
    );
}
