use rusqlite::{params, Connection};

use super::super::policy::{ContextLimits, ContextPolicy};
use super::super::query::load_context_data_with_policy;
use super::{insert_global_memory, insert_memory, setup_context_schema};

#[test]
fn load_context_data_uses_hybrid_retrieval_from_workstream_signal() {
    let conn = Connection::open_in_memory().unwrap();
    setup_context_schema(&conn);
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();
    let limits = ContextLimits {
        candidate_fetch_limit: 3,
        memory_index_limit: 10,
        core_item_limit: 4,
        ..ContextLimits::default()
    };
    let policy = ContextPolicy::from_limits(limits);

    for idx in 0..20 {
        insert_memory(
            &conn,
            idx + 1,
            project,
            Some(&format!("recent-noise-{idx}")),
            "discovery",
            &format!("Recent unrelated note {idx}"),
            "Recent context entry without the task terms.",
            now - idx,
        );
    }
    insert_memory(
        &conn,
        200,
        project,
        Some("sqlcipher-storage-decision"),
        "decision",
        "SQLCipher storage decision",
        "Persist private data with SQLCipher encryption at rest.",
        now - 10_000,
    );
    conn.execute(
        "INSERT INTO workstreams
         (id, project, title, status, next_action, created_at_epoch, updated_at_epoch)
         VALUES (1, ?1, 'Private persistence', 'active',
                 'Fix SQLCipher recall for private persisted data', ?2, ?2)",
        params![project, now],
    )
    .unwrap();

    let loaded = load_context_data_with_policy(&conn, project, None, &policy, true);

    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "SQLCipher storage decision"));
}

#[test]
fn hybrid_context_retrieval_still_excludes_global_non_preferences() {
    let conn = Connection::open_in_memory().unwrap();
    setup_context_schema(&conn);
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();
    let limits = ContextLimits {
        candidate_fetch_limit: 1,
        memory_index_limit: 10,
        core_item_limit: 3,
        ..ContextLimits::default()
    };
    let policy = ContextPolicy::from_limits(limits);

    insert_memory(
        &conn,
        1,
        project,
        Some("local-sqlcipher-decision"),
        "decision",
        "Local SQLCipher decision",
        "Repository-local SQLCipher storage decision.",
        now - 100,
    );
    insert_memory(
        &conn,
        2,
        "global",
        Some("global-sqlcipher-decision"),
        "bugfix",
        "Global SQLCipher note",
        "Global SQLCipher note should not enter project startup context.",
        now,
    );
    conn.execute(
        "UPDATE memories SET scope = 'global', owner_scope = 'user', owner_key = 'manual'
         WHERE id = 2",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO workstreams
         (id, project, title, status, next_action, created_at_epoch, updated_at_epoch)
         VALUES (1, ?1, 'SQLCipher recall', 'active',
                 'Find SQLCipher startup context decision', ?2, ?2)",
        params![project, now],
    )
    .unwrap();

    let loaded = load_context_data_with_policy(&conn, project, None, &policy, true);

    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Local SQLCipher decision"));
    assert!(!loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Global SQLCipher note"));
}

#[test]
fn hybrid_context_vector_recall_is_not_crowded_out_by_global_hits() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_context_schema(&conn);
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();
    let limits = ContextLimits {
        candidate_fetch_limit: 1,
        memory_index_limit: 10,
        core_item_limit: 3,
        ..ContextLimits::default()
    };
    let policy = ContextPolicy::from_limits(limits);

    insert_memory(
        &conn,
        1,
        project,
        Some("credential-store"),
        "architecture",
        "Credential store",
        "SQLCipher encrypts secrets at rest.",
        now - 10_000,
    );
    crate::retrieval::vector::upsert_memory_embedding_for_row(&conn, 1)?;
    for idx in 0..30 {
        let id = idx + 2;
        insert_global_memory(
            &conn,
            id,
            "global",
            Some(&format!("global-private-data-{idx}")),
            "bugfix",
            &format!("Global private data note {idx}"),
            "Protect private persisted data with a global-only diagnostic note.",
            now + idx,
        );
        crate::retrieval::vector::upsert_memory_embedding_for_row(&conn, id)?;
    }
    conn.execute(
        "INSERT INTO workstreams
         (id, project, title, status, next_action, created_at_epoch, updated_at_epoch)
         VALUES (1, ?1, 'Private persistence', 'active',
                 'How do we protect private persisted data?', ?2, ?2)",
        params![project, now],
    )?;

    let loaded = load_context_data_with_policy(&conn, project, None, &policy, true);

    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Credential store"));
    assert!(!loaded
        .memories
        .iter()
        .any(|memory| memory.title.starts_with("Global private data note")));
    Ok(())
}
