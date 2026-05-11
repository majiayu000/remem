use rusqlite::{params, Connection};

use super::{search_memories_fts_filtered, search_memories_like_filtered};
use crate::memory::tests_helper::setup_memory_schema;
use crate::memory::{insert_memory, search_memories_fts, search_memories_like};

#[test]
fn test_memory_fts_search() {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
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
    .expect("first memory should insert");
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
    .expect("second memory should insert");

    let results = search_memories_fts(&conn, "trigram", Some("proj"), None, 10, 0)
        .expect("fts search should succeed");
    assert_eq!(results.len(), 1);
    assert!(results[0].title.contains("trigram"));
}

#[test]
fn test_memory_like_fallback() {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
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
    .expect("memory should insert");

    let results = search_memories_like(&conn, &["DB"], Some("proj"), None, 10, 0)
        .expect("like search should succeed");
    assert_eq!(results.len(), 1);
    assert!(results[0].title.contains("DB"));
}

#[test]
fn search_memories_filtered_respects_branch_and_active_state() {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    setup_memory_schema(&conn);

    let main_id = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        None,
        "Main branch trigram",
        "content alpha",
        "decision",
        None,
    )
    .expect("main memory should insert");
    let null_branch_id = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        None,
        "Shared branch trigram",
        "content alpha",
        "decision",
        None,
    )
    .expect("shared memory should insert");
    let archived_id = insert_memory(
        &conn,
        Some("s1"),
        "proj",
        None,
        "Archived trigram",
        "content alpha",
        "decision",
        None,
    )
    .expect("archived memory should insert");

    conn.execute(
        "UPDATE memories SET branch = 'main' WHERE id = ?1",
        params![main_id],
    )
    .expect("main branch should update");
    conn.execute(
        "UPDATE memories SET branch = 'feature', status = 'archived' WHERE id = ?1",
        params![archived_id],
    )
    .expect("archived memory should update");

    let fts_results = search_memories_fts_filtered(
        &conn,
        "trigram",
        Some("proj"),
        Some("decision"),
        10,
        0,
        false,
        Some("main"),
    )
    .expect("filtered fts search should succeed");
    assert_eq!(fts_results.len(), 2);
    assert_eq!(fts_results[0].id, main_id);
    assert_eq!(fts_results[1].id, null_branch_id);

    let like_results = search_memories_like_filtered(
        &conn,
        &["alpha"],
        Some("proj"),
        Some("decision"),
        10,
        0,
        true,
        Some("feature"),
    )
    .expect("filtered like search should succeed");
    assert_eq!(like_results.len(), 2);
    let mut like_ids = like_results
        .iter()
        .map(|memory| memory.id)
        .collect::<Vec<_>>();
    like_ids.sort_unstable();
    assert_eq!(like_ids, vec![null_branch_id, archived_id]);
}
