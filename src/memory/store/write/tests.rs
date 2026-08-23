use super::*;
use crate::memory::get_recent_memories;
use crate::memory::tests_helper::setup_memory_schema;
use crate::retrieval::entity::search_by_entity;
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
fn test_topic_key_upsert_refreshes_entity_links() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);

    let id1 = insert_memory(
        &conn,
        Some("s1"),
        "test/proj",
        Some("server-stack"),
        "SQLCipher stack",
        "SQLCipher stores local data.",
        "decision",
        None,
    )?;
    let id2 = insert_memory(
        &conn,
        Some("s2"),
        "test/proj",
        Some("server-stack"),
        "Axum stack",
        "Axum handles the service layer.",
        "decision",
        None,
    )?;

    assert_eq!(id1, id2);
    assert!(search_by_entity(&conn, "SQLCipher", Some("test/proj"), 10)?.is_empty());
    assert_eq!(
        search_by_entity(&conn, "Axum", Some("test/proj"), 10)?,
        vec![id1]
    );

    let sqlcipher_count: i64 = conn.query_row(
        "SELECT mention_count FROM entities WHERE canonical_name = ?1 COLLATE NOCASE",
        params!["SQLCipher"],
        |row| row.get(0),
    )?;
    assert_eq!(sqlcipher_count, 0);
    Ok(())
}

#[test]
fn test_topic_key_upsert_with_no_entities_clears_entity_links() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);

    let id1 = insert_memory(
        &conn,
        Some("s1"),
        "test/proj",
        Some("plain-note"),
        "SQLCipher note",
        "SQLCipher stores local data.",
        "decision",
        None,
    )?;
    let id2 = insert_memory(
        &conn,
        Some("s2"),
        "test/proj",
        Some("plain-note"),
        "note",
        "plain words only",
        "decision",
        None,
    )?;

    assert_eq!(id1, id2);
    assert!(search_by_entity(&conn, "SQLCipher", Some("test/proj"), 10)?.is_empty());

    let link_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_entities WHERE memory_id = ?1",
        params![id1],
        |row| row.get(0),
    )?;
    assert_eq!(link_count, 0);
    Ok(())
}

#[test]
fn test_hash_like_ascii_preference_uses_existing_state_memory() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);

    let id1 = insert_memory_full(
        &conn,
        Some("s1"),
        "test/proj",
        Some("preference-11111111"),
        "Preference",
        "Keep verification status separate from data and code changes.",
        "preference",
        None,
        None,
        "global",
        None,
    )?;
    let id2 = insert_memory_full(
        &conn,
        Some("s2"),
        "test/proj",
        Some("preference-22222222"),
        "Preference",
        "Report data and code changes separately from verification status.",
        "preference",
        None,
        None,
        "global",
        None,
    )?;

    assert_eq!(id1, id2);
    let (topic_key, state_key, current_memory_id): (String, String, i64) = conn.query_row(
        "SELECT m.topic_key, sk.state_key, sk.current_memory_id
             FROM memories m
             JOIN memory_state_keys sk ON sk.id = m.state_key_id
             WHERE m.id = ?1",
        params![id1],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(topic_key, "preference-22222222");
    assert_eq!(state_key, "verification-status-separation");
    assert_eq!(current_memory_id, id1);

    let active_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE status = 'active'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(active_count, 1);
    Ok(())
}

#[test]
fn rust_api_repeat_preserves_verified_provenance() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    conn.execute(
        "INSERT INTO memory_candidates
         (id, scope, memory_type, topic_key, text, evidence_event_ids, confidence,
          risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (501, 'global', 'preference', 'verified-repeat',
                 'Prefer concise verified progress updates.', '[]', 0.95,
                 'low', 'approved', 1, 1)",
        [],
    )?;
    let memory_id = insert_memory_full(
        &conn,
        None,
        "project-a",
        Some("verified-repeat"),
        "Verified preference",
        "Prefer concise verified progress updates.",
        "preference",
        None,
        None,
        "global",
        None,
    )?;
    conn.execute(
        "UPDATE memories
         SET source_trust_class = 'user_prompt', source_candidate_id = 501,
             evidence_event_ids = '[]'
         WHERE id = ?1",
        [memory_id],
    )?;

    let repeated_id = insert_memory_full(
        &conn,
        None,
        "project-a",
        Some("verified-repeat"),
        "Verified preference",
        "  prefer concise VERIFIED progress updates.  ",
        "preference",
        None,
        None,
        "global",
        None,
    )?;
    assert_eq!(repeated_id, memory_id);
    assert_eq!(
        conn.query_row(
            "SELECT source_trust_class, source_candidate_id, evidence_event_ids
             FROM memories WHERE id = ?1",
            [memory_id],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?
            )),
        )?,
        ("user_prompt".to_string(), 501, "[]".to_string())
    );

    let rewritten_id = insert_memory_full(
        &conn,
        None,
        "project-a",
        Some("verified-repeat"),
        "Changed identity",
        "prefer concise verified progress updates.",
        "preference",
        None,
        None,
        "global",
        None,
    )?;
    assert_eq!(rewritten_id, memory_id);
    assert_eq!(
        conn.query_row(
            "SELECT source_trust_class, source_candidate_id, evidence_event_ids
             FROM memories WHERE id = ?1",
            [memory_id],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<String>>(2)?
            )),
        )?,
        ("local_tool_output".to_string(), None, None)
    );
    Ok(())
}

#[test]
fn rust_api_global_repeat_binds_the_existing_source_project_route() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    conn.execute(
        "INSERT INTO memories
         (project, topic_key, title, content, memory_type, created_at_epoch,
          updated_at_epoch, status, scope, source_project, owner_scope, owner_key,
          context_class, source_trust_class)
         VALUES ('project-a', 'legacy-global-topic', 'Legacy global fact',
                 'A shared global fact.', 'discovery', 1, 1, 'active', 'global',
                 'project-a', 'user', 'user:default', 'startup_core', 'local_tool_output')",
        [],
    )?;
    let first_id = conn.last_insert_rowid();
    let second_id = insert_memory_full(
        &conn,
        None,
        "project-b",
        Some("legacy-global-topic"),
        "Legacy global fact",
        "A shared global fact.",
        "discovery",
        None,
        None,
        "global",
        None,
    )?;
    assert_eq!(second_id, first_id);
    assert_eq!(
        conn.query_row(
            "SELECT project, source_project FROM memories WHERE id = ?1",
            [first_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?,
        ("project-a".to_string(), "project-a".to_string())
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE scope = 'global' AND topic_key = 'legacy-global-topic'",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        1
    );
    Ok(())
}

#[test]
fn test_hash_like_preference_upsert_clears_obsolete_state_keys() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);

    let id1 = insert_memory_full(
        &conn,
        Some("s1"),
        "test/proj",
        Some("preference-11111111"),
        "Preference",
        "Keep verification status separate from data and code changes.",
        "preference",
        None,
        None,
        "global",
        None,
    )?;
    let id2 = insert_memory_full(
        &conn,
        Some("s2"),
        "test/proj",
        Some("preference-11111111"),
        "Preference",
        "Keep data and code changes separate in reports.",
        "preference",
        None,
        None,
        "global",
        None,
    )?;

    assert_eq!(id1, id2);

    let old_current: Option<i64> = conn.query_row(
        "SELECT current_memory_id FROM memory_state_keys
             WHERE state_key = 'verification-status-separation'",
        [],
        |row| row.get::<_, Option<i64>>(0),
    )?;
    let new_current: Option<i64> = conn.query_row(
        "SELECT current_memory_id FROM memory_state_keys
             WHERE state_key = 'data-code-change-separation'",
        [],
        |row| row.get::<_, Option<i64>>(0),
    )?;
    assert_eq!(old_current, None);
    assert_eq!(new_current, Some(id1));

    let id3 = insert_memory_full(
        &conn,
        Some("s3"),
        "test/proj",
        Some("preference-11111111"),
        "Preference",
        "Prefer concise progress updates.",
        "preference",
        None,
        None,
        "global",
        None,
    )?;
    assert_eq!(id1, id3);

    let state_key_id: Option<i64> = conn.query_row(
        "SELECT state_key_id FROM memories WHERE id = ?1",
        params![id1],
        |row| row.get(0),
    )?;
    let current_slots: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_state_keys WHERE current_memory_id = ?1",
        params![id1],
        |row| row.get(0),
    )?;
    assert_eq!(state_key_id, None);
    assert_eq!(current_slots, 0);
    Ok(())
}

#[test]
fn test_hash_like_cjk_preference_uses_existing_state_memory() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);

    let id1 = insert_memory_full(
        &conn,
        Some("s1"),
        "test/proj",
        Some("preference-aaaaaaaa"),
        "Preference",
        "验证状态必须和数据、代码变更分开说明。",
        "preference",
        None,
        None,
        "global",
        None,
    )?;
    let id2 = insert_memory_full(
        &conn,
        Some("s2"),
        "test/proj",
        Some("preference-bbbbbbbb"),
        "Preference",
        "用户要求数据和代码变更要与验证状态分离报告。",
        "preference",
        None,
        None,
        "global",
        None,
    )?;

    assert_eq!(id1, id2);
    let state_key: String = conn.query_row(
        "SELECT sk.state_key
             FROM memories m
             JOIN memory_state_keys sk ON sk.id = m.state_key_id
             WHERE m.id = ?1",
        params![id1],
        |row| row.get(0),
    )?;
    assert_eq!(state_key, "verification-status-separation");
    Ok(())
}

#[test]
fn test_same_state_key_text_keeps_distinct_owner_rows() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let content = "Keep verification status separate from data and code changes.";

    let user_id = insert_memory_full(
        &conn,
        Some("s1"),
        "test/proj",
        Some("preference-11111111"),
        "Preference",
        content,
        "preference",
        None,
        None,
        "global",
        None,
    )?;
    let repo_id = insert_memory_full(
        &conn,
        Some("s2"),
        "test/proj",
        Some("preference-22222222"),
        "Preference",
        content,
        "preference",
        None,
        None,
        "project",
        None,
    )?;

    assert_ne!(user_id, repo_id);
    let state_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_state_keys
             WHERE state_key = 'verification-status-separation'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(state_rows, 2);
    Ok(())
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

    let row: (i64, i64, i64) = conn
        .query_row(
            "SELECT created_at_epoch, updated_at_epoch, reference_time_epoch FROM memories WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(row.0, custom_epoch);
    assert_ne!(row.1, custom_epoch);
    assert_eq!(row.2, custom_epoch);
}

#[test]
fn test_reference_time_override() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let created_at: i64 = 1_700_000_000;
    let reference_time: i64 = 1_600_000_000;
    let id = insert_memory_full_with_reference_time(
        &conn,
        Some("s1"),
        "proj",
        None,
        "Imported event",
        "Yesterday meant the historical episode date.",
        "discovery",
        None,
        None,
        "project",
        Some(created_at),
        Some(reference_time),
    )
    .unwrap();

    let row: (i64, i64) = conn
        .query_row(
            "SELECT created_at_epoch, reference_time_epoch FROM memories WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(row.0, created_at);
    assert_eq!(row.1, reference_time);
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

#[test]
fn insert_memory_writes_embedding() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);

    let id = insert_memory(
        &conn,
        Some("s1"),
        "test/proj",
        Some("semantic-recall"),
        "Semantic recall",
        "Vector search retrieves paraphrased memory.",
        "decision",
        None,
    )?;

    let row: (i64, i64, String) = conn.query_row(
        "SELECT memory_id, dimensions, model FROM memory_embeddings WHERE memory_id = ?1",
        params![id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(row.0, id);
    assert_eq!(row.1, crate::retrieval::vector::EMBEDDING_DIMENSIONS as i64);
    assert_eq!(row.2, crate::retrieval::vector::DEFAULT_EMBEDDING_MODEL);
    Ok(())
}

#[test]
fn topic_key_upsert_replaces_embedding_for_same_memory() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);

    let id = insert_memory(
        &conn,
        Some("s1"),
        "test/proj",
        Some("semantic-recall"),
        "Semantic recall",
        "Initial vector note.",
        "decision",
        None,
    )?;
    let before_hash: String = conn.query_row(
        "SELECT content_hash FROM memory_embeddings WHERE memory_id = ?1",
        params![id],
        |row| row.get(0),
    )?;

    let updated_id = insert_memory(
        &conn,
        Some("s2"),
        "test/proj",
        Some("semantic-recall"),
        "Semantic recall",
        "Updated vector note with different content.",
        "decision",
        None,
    )?;
    assert_eq!(updated_id, id);

    let row: (i64, String) = conn.query_row(
        "SELECT COUNT(*), MAX(content_hash) FROM memory_embeddings WHERE memory_id = ?1",
        params![id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(row.0, 1);
    assert_ne!(row.1, before_hash);
    Ok(())
}
