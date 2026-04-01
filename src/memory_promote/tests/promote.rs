use super::super::promote::promote_summary_to_memories;
use super::super::slug::content_hash;
use crate::memory::tests_helper::setup_memory_schema;

#[test]
fn test_promote_multi_decisions() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let decisions = "• Use RwLock instead of Mutex for concurrent read support\n\
                     • Switch to trigram tokenizer for CJK text search\n\
                     • Set compression threshold to 100 observations";
    let count = promote_summary_to_memories(
        &conn,
        "session-1",
        "test/proj",
        Some("Optimize search and concurrency"),
        Some(decisions),
        None,
        None,
    )
    .unwrap();
    assert_eq!(count, 3);

    let memories = crate::memory::get_recent_memories(&conn, "test/proj", 10).unwrap();
    let titles: Vec<&str> = memories
        .iter()
        .map(|memory| memory.title.as_str())
        .collect();
    assert!(
        titles.iter().any(|title| title.contains("RwLock")),
        "title should contain keyword from content: {:?}",
        titles
    );
    assert!(
        titles.iter().any(|title| title.contains("trigram")),
        "title should contain keyword from content: {:?}",
        titles
    );
}

#[test]
fn test_promote_multi_learned() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let learned = "- FTS5 trigram tokenizer handles CJK without word boundaries\n\
                   - WAL mode allows concurrent reads with single writer";
    let count = promote_summary_to_memories(
        &conn,
        "session-1",
        "test/proj",
        Some("Research storage"),
        None,
        Some(learned),
        None,
    )
    .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn test_promote_content_format() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let decisions = "Switched from unicode61 to trigram tokenizer for better CJK support";
    promote_summary_to_memories(
        &conn,
        "session-1",
        "test/proj",
        Some("Fix search"),
        Some(decisions),
        None,
        None,
    )
    .unwrap();

    let memories = crate::memory::get_recent_memories(&conn, "test/proj", 10).unwrap();
    assert_eq!(memories.len(), 1);
    assert!(
        !memories[0].text.contains("**Request**"),
        "content should not have boilerplate: {}",
        memories[0].text
    );
    assert!(
        memories[0].text.contains("[Context:"),
        "content should have compact context: {}",
        memories[0].text
    );
}

#[test]
fn test_content_hash_dedup() {
    let hash1 = content_hash("Use FTS5 trigram tokenizer for CJK support");
    let hash2 = content_hash("Use FTS5 trigram tokenizer for CJK support");
    assert_eq!(hash1, hash2);

    let hash3 = content_hash("Switch to WAL mode for concurrent reads");
    assert_ne!(hash1, hash3);
}

#[test]
fn test_cross_session_dedup() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let decision = "Use FTS5 trigram tokenizer for CJK text search support";
    promote_summary_to_memories(
        &conn,
        "session-1",
        "test/proj",
        Some("Optimize search"),
        Some(decision),
        None,
        None,
    )
    .unwrap();

    promote_summary_to_memories(
        &conn,
        "session-2",
        "test/proj",
        Some("Database performance tuning"),
        Some(decision),
        None,
        None,
    )
    .unwrap();

    let memories = crate::memory::get_recent_memories(&conn, "test/proj", 10).unwrap();
    assert_eq!(
        memories.len(),
        1,
        "same decision should dedup across sessions"
    );
}
