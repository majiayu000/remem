use super::*;
use crate::memory::tests_helper::setup_memory_schema;
use crate::memory::{
    insert_memory, search_memories_fts, search_memories_fts_filtered, search_memories_like,
};
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

fn insert_fts_fixture_memory(conn: &Connection, id: i64, title: &str, status: &str) {
    conn.execute(
        "INSERT INTO memories (
            id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status
         ) VALUES (?1, 'proj', ?2, ?2, 'decision', 100, 100, ?3)",
        rusqlite::params![id, title, status],
    )
    .unwrap();
}

#[test]
fn fixture_fts_indexes_all_statuses_and_filters_visibility_in_queries() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    insert_fts_fixture_memory(&conn, 1, "zookeeper active", "active");
    insert_fts_fixture_memory(&conn, 2, "zookeeper stale", "stale");
    insert_fts_fixture_memory(&conn, 3, "zookeeper archived", "archived");

    let active_only =
        search_memories_fts_filtered(&conn, "zookeeper", Some("proj"), None, 10, 0, false, None)
            .unwrap();
    assert_eq!(
        active_only
            .iter()
            .map(|memory| memory.id)
            .collect::<Vec<_>>(),
        vec![1]
    );

    let all_statuses =
        search_memories_fts_filtered(&conn, "zookeeper", Some("proj"), None, 10, 0, true, None)
            .unwrap();
    let mut ids = all_statuses
        .iter()
        .map(|memory| memory.id)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[test]
fn fixture_fts_preserves_rows_across_status_transitions_and_delete() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    insert_fts_fixture_memory(&conn, 1, "cassandra transition", "active");

    conn.execute("UPDATE memories SET status = 'stale' WHERE id = 1", [])
        .unwrap();
    let stale =
        search_memories_fts_filtered(&conn, "cassandra", Some("proj"), None, 10, 0, true, None)
            .unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].status, "stale");
    assert!(search_memories_fts_filtered(
        &conn,
        "cassandra",
        Some("proj"),
        None,
        10,
        0,
        false,
        None,
    )
    .unwrap()
    .is_empty());

    conn.execute("UPDATE memories SET status = 'active' WHERE id = 1", [])
        .unwrap();
    assert_eq!(
        search_memories_fts_filtered(&conn, "cassandra", Some("proj"), None, 10, 0, false, None,)
            .unwrap()
            .len(),
        1
    );

    conn.execute("DELETE FROM memories WHERE id = 1", [])
        .unwrap();
    assert!(search_memories_fts_filtered(
        &conn,
        "cassandra",
        Some("proj"),
        None,
        10,
        0,
        true,
        None
    )
    .unwrap()
    .is_empty());
}

fn memory_fts_trigger_sql(conn: &Connection) -> Vec<(String, String)> {
    let mut stmt = conn
        .prepare(
            "SELECT name, sql FROM sqlite_master
             WHERE type = 'trigger' AND name IN ('memories_ai', 'memories_au', 'memories_ad')
             ORDER BY name",
        )
        .unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

#[test]
fn fixture_fts_triggers_match_canonical_migrations() {
    let fixture = Connection::open_in_memory().unwrap();
    setup_memory_schema(&fixture);

    let migrated = Connection::open_in_memory().unwrap();
    crate::migrate::run_migrations(&migrated).unwrap();

    assert_eq!(
        memory_fts_trigger_sql(&fixture),
        memory_fts_trigger_sql(&migrated)
    );
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
