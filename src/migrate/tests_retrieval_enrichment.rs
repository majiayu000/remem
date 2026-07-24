//! v072 memory_retrieval_enrichment migration tests (GH-850).

use rusqlite::{params, Connection};

fn migrated_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::migrate::run_migrations(&conn).unwrap();
    crate::retrieval::vector::ensure_vec_table(&conn).unwrap();
    conn
}

fn fts_matches(conn: &Connection, term: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH ?1",
        params![term],
        |row| row.get(0),
    )
    .unwrap()
}

fn assert_fts_integrity_clean(conn: &Connection) {
    conn.execute(
        "INSERT INTO memories_fts(memories_fts, rank) VALUES ('integrity-check', 1)",
        [],
    )
    .expect("memories_fts must stay consistent with the memories content table");
}

#[test]
fn migration_adds_pending_defaults_and_ready_singleton() {
    let conn = migrated_conn();
    conn.execute(
        "INSERT INTO memories (id, project, title, content, memory_type,
             created_at_epoch, updated_at_epoch, status)
         VALUES (1, 'proj', 'T', 'body', 'decision', 1, 1, 'active')",
        [],
    )
    .unwrap();
    let (version, policy, attempt, failures): (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT search_context_enrichment_version, search_context_security_policy_version,
                    search_context_enrichment_attempt, search_context_failure_count
             FROM memories WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!((version, policy, attempt, failures), (0, 0, 0, 0));

    let (floor, epoch, target, state): (i64, i64, i64, String) = conn
        .query_row(
            "SELECT min_security_policy_version, compatibility_epoch,
                    target_security_policy_version, convergence_state
             FROM retrieval_enrichment_compatibility WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!((floor, epoch, target, state.as_str()), (1, 1, 1, "ready"));
}

/// B-009 sequence: raw canonical UPDATE -> the same transaction persists the
/// empty fallback and invalidates identity -> an unrelated access_count UPDATE
/// cannot restore the old enrichment -> old term MATCH 0, new canonical term
/// hits, old vector removed, FTS integrity clean.
#[test]
fn raw_canonical_update_invalidates_enrichment_and_vectors() {
    let conn = migrated_conn();
    conn.execute(
        "INSERT INTO memories (id, project, title, content, memory_type,
             created_at_epoch, updated_at_epoch, status)
         VALUES (1, 'proj', 'T', 'old canonical body', 'decision', 1, 1, 'active')",
        [],
    )
    .unwrap();
    // Simulate a ready enrichment with an enrichment-only term and a vector.
    conn.execute(
        "UPDATE memories SET
             search_context = 'context: recall helper\nkeywords: zebrafishterm',
             search_context_enrichment_version = 1,
             search_context_security_policy_version = 1,
             search_context_source_hash = 'srchash',
             search_context_fallback_source_hash = 'srchash',
             search_context_index_hash = 'idxhash'
         WHERE id = 1",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memory_embeddings
             (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
         VALUES (1, x'0000803f', 1, 'test-model', 'idxhash', 1)",
        [],
    )
    .unwrap();
    assert_eq!(fts_matches(&conn, "zebrafishterm"), 1);

    // Raw canonical update without resetting the fallback source hash.
    conn.execute(
        "UPDATE memories SET content = 'new canonical wording' WHERE id = 1",
        [],
    )
    .unwrap();

    let (search_context, version, policy, source, fallback, index, vectors): (
        String,
        i64,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
    ) = conn
        .query_row(
            "SELECT search_context, search_context_enrichment_version,
                    search_context_security_policy_version, search_context_source_hash,
                    search_context_fallback_source_hash, search_context_index_hash,
                    (SELECT COUNT(*) FROM memory_embeddings WHERE memory_id = 1)
             FROM memories WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        search_context, "",
        "empty fallback must be persisted on the row"
    );
    assert_eq!(version, 0);
    assert_eq!(policy, 1, "policy resets to the DB floor");
    assert!(source.is_none() && fallback.is_none() && index.is_none());
    assert_eq!(vectors, 0, "stale vector must be removed immediately");

    // Unrelated update must not re-introduce the old enrichment text.
    conn.execute(
        "UPDATE memories SET access_count = access_count + 1 WHERE id = 1",
        [],
    )
    .unwrap();
    assert_eq!(
        fts_matches(&conn, "zebrafishterm"),
        0,
        "old term must stay dead"
    );
    assert_eq!(
        fts_matches(&conn, "canonical wording"),
        1,
        "new canonical term must hit"
    );
    assert_fts_integrity_clean(&conn);
}

/// Positive case: a production writer that resets the fallback source hash in
/// the same statement is not treated as a bypass write, and its deterministic
/// search context stays visible.
#[test]
fn production_writer_same_statement_reset_keeps_deterministic_fallback() {
    let conn = migrated_conn();
    let id = crate::memory::insert_memory(
        &conn,
        None,
        "proj",
        Some("gh850-writer"),
        "Cache fix",
        "Issue: cache timeout. Resolved by invalidating stale entries.",
        "bugfix",
        None,
    )
    .unwrap();
    let updated = crate::memory::insert_memory(
        &conn,
        None,
        "proj",
        Some("gh850-writer"),
        "Cache fix",
        "Issue: cache timeout. Resolved by rotating cache keys instead.",
        "bugfix",
        None,
    )
    .unwrap();
    assert_eq!(id, updated, "same topic key must update the same row");

    let (search_context, fallback): (String, Option<String>) = conn
        .query_row(
            "SELECT search_context, search_context_fallback_source_hash
             FROM memories WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(
        search_context.contains("rotating cache keys"),
        "deterministic hints must survive the production update: {search_context:?}"
    );
    let expected = crate::memory::retrieval_enrichment::enrichment_source_hash(
        "Cache fix",
        "Issue: cache timeout. Resolved by rotating cache keys instead.",
        "bugfix",
        Some("gh850-writer"),
        None,
    );
    assert_eq!(fallback.as_deref(), Some(expected.as_str()));
    assert_fts_integrity_clean(&conn);
}

#[test]
fn v072_is_latest_and_named_stably() {
    let migration = super::types::MIGRATIONS.last().unwrap();
    assert_eq!(migration.version, 72);
    assert_eq!(migration.name, "memory_retrieval_enrichment");
}
