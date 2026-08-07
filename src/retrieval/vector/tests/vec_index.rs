use rusqlite::params;

use super::super::vec_index::{
    ensure_vec_index, knn_candidates, sync_vec_keep_only_profile, vec_index_ready,
};
use super::*;

fn setup_vec_conn() -> Result<VectorTestConn> {
    let ctx = setup_vector_conn()?;
    load_vec_extension(&ctx.conn)?;
    Ok(ctx)
}

fn insert_embedded_memory(conn: &Connection, id: i64, title: &str, content: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
         VALUES (?1, '/repo', ?2, ?3, 'architecture', 1, 1, 'active')",
        params![id, title, content],
    )?;
    upsert_memory_embedding(conn, id, title, content, "architecture", None, "")
}

fn served_by_knn(outcome: &VectorSearchOutcome) -> bool {
    outcome
        .timings
        .iter()
        .all(|timing| timing.phase != "vector_select_candidates")
}

#[test]
fn extension_loads_and_reports_version() -> Result<()> {
    let ctx = setup_vec_conn()?;
    assert!(vec_extension_loaded(&ctx.conn));
    // Idempotent on an already-initialized connection.
    load_vec_extension(&ctx.conn)?;
    assert!(vec_extension_loaded(&ctx.conn));
    Ok(())
}

#[test]
fn knn_unavailable_without_extension_or_backfill() -> Result<()> {
    let ctx = setup_vector_conn()?;
    insert_embedded_memory(&ctx.conn, 1, "SQLCipher", "Secrets are encrypted at rest.")?;

    let query = embed_query_text("encrypted secrets");
    let profile = TextEmbedding::new(DEFAULT_EMBEDDING_MODEL, query.clone())?;
    // No extension on this connection: KNN answers None, search still works.
    assert!(knn_candidates(
        &ctx.conn,
        &query,
        profile.profile(),
        VectorSearchFilters::default(),
        16,
    )?
    .is_none());

    load_vec_extension(&ctx.conn)?;
    // Extension present but no backfill yet: still None.
    assert!(!vec_index_ready(&ctx.conn, profile.profile())?);
    assert!(knn_candidates(
        &ctx.conn,
        &query,
        profile.profile(),
        VectorSearchFilters::default(),
        16,
    )?
    .is_none());

    let outcome = vector_search_filtered(&ctx.conn, &query, VectorSearchFilters::default(), 5)?;
    assert_eq!(outcome.hits[0].memory_id, 1);
    assert!(!served_by_knn(&outcome));
    Ok(())
}

#[test]
fn knn_path_matches_brute_force_ranking() -> Result<()> {
    let ctx = setup_vec_conn()?;
    insert_embedded_memory(&ctx.conn, 1, "SQLCipher", "Secrets are encrypted at rest.")?;
    insert_embedded_memory(&ctx.conn, 2, "Retrieval", "Vector search ranks memories.")?;
    insert_embedded_memory(&ctx.conn, 3, "Hooks", "SessionStart injects context.")?;

    let query = embed_query_text("how are secrets protected");
    let brute = vector_search_filtered(&ctx.conn, &query, VectorSearchFilters::default(), 3)?;
    assert!(!served_by_knn(&brute));

    ensure_vec_index(&ctx.conn)?;
    let profile = TextEmbedding::new(DEFAULT_EMBEDDING_MODEL, query.clone())?;
    assert!(vec_index_ready(&ctx.conn, profile.profile())?);

    let indexed = vector_search_filtered(&ctx.conn, &query, VectorSearchFilters::default(), 3)?;
    assert!(served_by_knn(&indexed));
    let brute_ids: Vec<i64> = brute.hits.iter().map(|hit| hit.memory_id).collect();
    let indexed_ids: Vec<i64> = indexed.hits.iter().map(|hit| hit.memory_id).collect();
    assert_eq!(brute_ids, indexed_ids);
    Ok(())
}

#[test]
fn knn_respects_memory_filters() -> Result<()> {
    let ctx = setup_vec_conn()?;
    for (id, project, branch, memory_type, status) in [
        (1, "/repo", Some("main"), "architecture", "active"),
        (2, "/other", Some("main"), "architecture", "active"),
        (3, "/repo", Some("feature"), "architecture", "active"),
        (4, "/repo", Some("main"), "decision", "active"),
        (5, "/repo", Some("main"), "architecture", "stale"),
    ] {
        ctx.conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status, branch)
             VALUES (?1, ?2, 'Credential store', 'SQLCipher encrypts secrets at rest.', ?3, 1, 1, ?4, ?5)",
            params![id, project, memory_type, status, branch],
        )?;
        upsert_memory_embedding(
            &ctx.conn,
            id,
            "Credential store",
            "SQLCipher encrypts secrets at rest.",
            memory_type,
            None,
            "",
        )?;
    }
    ensure_vec_index(&ctx.conn)?;

    let query = embed_query_text("protect private persisted data");
    let outcome = vector_search_filtered(
        &ctx.conn,
        &query,
        VectorSearchFilters {
            project: Some("/repo"),
            branch: Some("main"),
            memory_type: Some("architecture"),
            include_stale: false,
        },
        10,
    )?;
    assert!(served_by_knn(&outcome));
    let ids: Vec<i64> = outcome.hits.iter().map(|hit| hit.memory_id).collect();
    assert_eq!(ids, vec![1]);
    Ok(())
}

#[test]
fn dual_write_keeps_index_current_without_new_backfill() -> Result<()> {
    let ctx = setup_vec_conn()?;
    insert_embedded_memory(&ctx.conn, 1, "Older", "Unrelated placeholder note.")?;
    ensure_vec_index(&ctx.conn)?;

    // Written after backfill completed: only the dual-write path covers it.
    insert_embedded_memory(&ctx.conn, 2, "SQLCipher", "Secrets are encrypted at rest.")?;

    let query = embed_query_text("encrypted secrets at rest");
    let outcome = vector_search_filtered(&ctx.conn, &query, VectorSearchFilters::default(), 2)?;
    assert!(served_by_knn(&outcome));
    assert!(outcome.hits.iter().any(|hit| hit.memory_id == 2));
    Ok(())
}

#[test]
fn backfill_marks_profile_done_and_is_idempotent() -> Result<()> {
    let ctx = setup_vec_conn()?;
    insert_embedded_memory(&ctx.conn, 1, "One", "First memory body.")?;
    insert_embedded_memory(&ctx.conn, 2, "Two", "Second memory body.")?;

    ensure_vec_index(&ctx.conn)?;
    let query = embed_query_text("memory body");
    let profile = TextEmbedding::new(DEFAULT_EMBEDDING_MODEL, query)?;
    assert!(vec_index_ready(&ctx.conn, profile.profile())?);
    ensure_vec_index(&ctx.conn)?;
    assert!(vec_index_ready(&ctx.conn, profile.profile())?);

    let mirrored: i64 = ctx.conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM memory_embedding_vec_{}",
            profile.profile().dimensions
        ),
        [],
        |row| row.get(0),
    )?;
    assert_eq!(mirrored, 2);
    Ok(())
}

#[test]
fn keep_only_profile_drops_other_dimension_tables() -> Result<()> {
    let ctx = setup_vec_conn()?;
    insert_embedded_memory(&ctx.conn, 1, "One", "First memory body.")?;
    ensure_vec_index(&ctx.conn)?;
    let dimensions = EMBEDDING_DIMENSIONS;

    // Fabricate a stale mirror table from a different dimension profile.
    ctx.conn.execute_batch(
        "CREATE VIRTUAL TABLE memory_embedding_vec_8 USING vec0(
             memory_id INTEGER PRIMARY KEY,
             embedding float[8] distance_metric=cosine,
             +model TEXT
         )",
    )?;

    sync_vec_keep_only_profile(&ctx.conn, DEFAULT_EMBEDDING_MODEL, dimensions)?;
    let stale_exists: i64 = ctx.conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'memory_embedding_vec_8'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(stale_exists, 0);
    let active_exists: i64 = ctx.conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [format!("memory_embedding_vec_{dimensions}")],
        |row| row.get(0),
    )?;
    assert_eq!(active_exists, 1);
    Ok(())
}
