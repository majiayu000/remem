use rusqlite::params;

use super::*;

fn target() -> EmbeddingBackfillTarget {
    EmbeddingBackfillTarget {
        model: "test-source-profile".to_string(),
        dimensions: 2,
    }
}

fn insert_memory(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, project, title, content, memory_type, status, created_at_epoch, updated_at_epoch)
         VALUES (?1, '/repo', 'Database', 'Use MySQL', 'decision', 'active', 100, 100)",
        [id],
    )?;
    Ok(())
}

fn prepare_pending(conn: &Connection, limit: i64) -> Result<Vec<PreparedMemoryEmbedding>> {
    Ok(
        select_memory_embedding_reindex_candidates(conn, &target(), limit)?
            .into_iter()
            .map(|source| PreparedMemoryEmbedding {
                memory_id: source.id,
                model: target().model,
                content_hash: crate::retrieval::embedding::memory_index_hash(
                    &source.title,
                    &source.content,
                    &source.memory_type,
                    source.topic_key.as_deref(),
                    &source.search_context,
                ),
                values: vec![1.0, 0.0],
                // A later completion must not make an older passage authoritative.
                updated_at_epoch: 200,
            })
            .collect(),
    )
}

struct TestDirectory(std::path::PathBuf);

impl TestDirectory {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "remem-backfill-source-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("failed to remove backfill fixture: {error}");
        }
    }
}

#[test]
fn late_backfill_cannot_replace_foreground_vector_or_profile_mirror() -> Result<()> {
    let directory = TestDirectory::new()?;
    let path = directory.0.join("memory.sqlite");
    let backfill = Connection::open(&path)?;
    crate::migrate::run_migrations(&backfill)?;
    super::super::load_vec_extension(&backfill)?;
    insert_memory(&backfill, 1)?;
    let stale = prepare_pending(&backfill, 1)?;

    // A separate writer commits a new passage and its correct vector while
    // the first batch is outside SQLite preparing the old passage's vector.
    let writer = Connection::open(&path)?;
    super::super::load_vec_extension(&writer)?;
    writer.execute_batch("BEGIN IMMEDIATE")?;
    writer.execute(
        "UPDATE memories SET content = 'Use PostgreSQL', updated_at_epoch = 101 WHERE id = 1",
        [],
    )?;
    let mut current = prepare_pending(&writer, 1)?;
    current[0].values = vec![-1.0, 0.0];
    current[0].updated_at_epoch = 102;
    let current_hash = current[0].content_hash.clone();
    assert_eq!(
        upsert_prepared_memory_embedding_batch(&writer, &current, &mut vec![])?,
        1
    );
    writer.execute_batch("COMMIT")?;
    super::super::vec_index::ensure_vec_index(&writer)?;

    assert_eq!(
        upsert_prepared_memory_embedding_batch(&backfill, &stale, &mut vec![])?,
        0
    );
    let (stored_hash, completed): (String, i64) = backfill.query_row(
        "SELECT content_hash, updated_at_epoch FROM memory_embeddings WHERE memory_id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(stored_hash, current_hash);
    assert_eq!(completed, 102);
    assert_eq!(
        pending_memory_embedding_reindex_count_for_target(&backfill, &target())?,
        0
    );
    assert!(prepare_pending(&backfill, 1)?.is_empty());
    let profile = target();
    let hits = super::super::vec_index::knn_candidates(
        &backfill,
        &[1.0, 0.0],
        crate::retrieval::embedding::EmbeddingProfile {
            model: &profile.model,
            dimensions: profile.dimensions,
        },
        super::super::VectorSearchFilters::default(),
        1,
    )?
    .expect("current profile mirror is ready");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].distance, 2.0);
    Ok(())
}

#[test]
fn pending_uses_source_hash_when_completion_is_newer_and_source_timestamp_is_equal() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    insert_memory(&conn, 1)?;
    let initial = prepare_pending(&conn, 1)?;
    upsert_prepared_memory_embedding_batch(&conn, &initial, &mut vec![])?;
    conn.execute(
        "UPDATE memories SET content = 'Use PostgreSQL' WHERE id = 1",
        [],
    )?;
    assert_eq!(
        pending_memory_embedding_reindex_count_for_target(&conn, &target())?,
        1
    );
    let changed = prepare_pending(&conn, 1)?;
    assert_eq!(changed.len(), 1);
    assert_ne!(changed[0].content_hash, initial[0].content_hash);
    assert_eq!(
        upsert_prepared_memory_embedding_batch(&conn, &changed, &mut vec![])?,
        1
    );
    assert_eq!(
        pending_memory_embedding_reindex_count_for_target(&conn, &target())?,
        0
    );

    // Enrichment changes the index passage without changing canonical time.
    conn.execute(
        "UPDATE memories SET search_context = 'relational storage',
             search_context_source_hash = 'ready' WHERE id = 1",
        [],
    )?;
    assert_eq!(
        pending_memory_embedding_reindex_count_for_target(&conn, &target())?,
        1
    );
    let enrichment = prepare_pending(&conn, 1)?;
    conn.execute(
        "UPDATE memories SET search_context = 'transactional storage' WHERE id = 1",
        [],
    )?;
    assert_eq!(
        upsert_prepared_memory_embedding_batch(&conn, &enrichment, &mut vec![])?,
        0
    );
    assert_eq!(
        pending_memory_embedding_reindex_count_for_target(&conn, &target())?,
        1
    );
    let current = prepare_pending(&conn, 1)?;
    assert_eq!(
        upsert_prepared_memory_embedding_batch(&conn, &current, &mut vec![])?,
        1
    );
    assert_eq!(
        pending_memory_embedding_reindex_count_for_target(&conn, &target())?,
        0
    );
    // Timestamp-only maintenance does not invalidate an identical passage.
    conn.execute(
        "UPDATE memories SET updated_at_epoch = 300 WHERE id = 1",
        [],
    )?;
    assert!(prepare_pending(&conn, 1)?.is_empty());
    Ok(())
}

#[test]
fn mixed_batch_skips_changed_deleted_and_ineligible_sources() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    for id in 1..=4 {
        insert_memory(&conn, id)?;
    }
    let prepared = prepare_pending(&conn, 4)?;
    assert_eq!(prepared.len(), 4);
    conn.execute(
        "UPDATE memories SET content = 'Use PostgreSQL' WHERE id = 1",
        [],
    )?;
    conn.execute("DELETE FROM memories WHERE id = 2", [])?;
    conn.execute("UPDATE memories SET status = 'superseded' WHERE id = 3", [])?;
    assert_eq!(
        upsert_prepared_memory_embedding_batch(&conn, &prepared, &mut vec![])?,
        1
    );
    let ids: Vec<i64> = conn
        .prepare("SELECT memory_id FROM memory_embeddings ORDER BY memory_id")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    assert_eq!(ids, vec![4]);
    assert_eq!(
        pending_memory_embedding_reindex_count_for_target(&conn, &target())?,
        1
    );
    // A current row ahead of the stale one cannot consume the batch limit.
    conn.execute(
        "UPDATE memories SET updated_at_epoch = 400 WHERE id = 4",
        [],
    )?;
    let pending = prepare_pending(&conn, 1)?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].memory_id, 1);
    assert_eq!(
        upsert_prepared_memory_embedding_batch(&conn, &pending, &mut vec![])?,
        1
    );
    assert_eq!(
        pending_memory_embedding_reindex_count_for_target(&conn, &target())?,
        0
    );
    let unchanged: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_embeddings WHERE memory_id IN (?1, ?2)",
        params![2, 3],
        |row| row.get(0),
    )?;
    assert_eq!(unchanged, 0);
    Ok(())
}
