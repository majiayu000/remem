use std::sync::mpsc;
use std::time::Duration;

use rusqlite::OptionalExtension;

use super::*;
use crate::retrieval::embedding::{EmbeddingBackfillTarget, EmbeddingFallbackCache};
use crate::retrieval::vector::backfill::upsert_prepared_memory_embedding_batch;
use crate::retrieval::vector::reindex::{
    prepare_memory_embedding_batch, select_memory_embedding_reindex_candidates,
    PreparedMemoryEmbedding,
};

fn target() -> EmbeddingBackfillTarget {
    EmbeddingBackfillTarget {
        model: DEFAULT_EMBEDDING_MODEL.to_string(),
        dimensions: EMBEDDING_DIMENSIONS,
    }
}

fn stored_embedding(conn: &Connection) -> Result<Option<(Vec<u8>, String, i64)>> {
    Ok(conn
        .query_row(
            "SELECT embedding, content_hash, updated_at_epoch FROM memory_embeddings
             WHERE memory_id = 1 AND model = ?1 AND dimensions = ?2",
            params![DEFAULT_EMBEDDING_MODEL, EMBEDDING_DIMENSIONS as i64],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?)
}

#[test]
fn reindex_source_cas_preserves_foreground_changes_across_sqlite_connections() -> Result<()> {
    let _provider = ScopedEmbeddingProvider::new("feature-hash");
    for (case, mutation, write_new_vector, expected_pending) in [
        ("canonical", "UPDATE memories SET content = 'PostgreSQL replaces MySQL for durable storage.', updated_at_epoch = 1 WHERE id = 1", true, 0),
        ("enrichment", "UPDATE memories SET search_context = 'context: new enrichment snapshot', search_context_source_hash = 'new-source', search_context_index_hash = 'do-not-trust-this-cache', updated_at_epoch = 1 WHERE id = 1", true, 0),
        ("canonical-pending", "UPDATE memories SET content = 'PostgreSQL replaces MySQL for durable storage.', updated_at_epoch = 1 WHERE id = 1", false, 1),
        ("enrichment-pending", "UPDATE memories SET search_context = 'context: new enrichment snapshot', search_context_source_hash = 'new-source', updated_at_epoch = 1 WHERE id = 1", false, 1),
        ("deleted", "DELETE FROM memories WHERE id = 1", false, 0),
        ("quarantined", "UPDATE memories SET status = 'quarantined' WHERE id = 1", false, 0),
    ] {
        let path = std::env::temp_dir().join(format!(
            "remem-reindex-cas-{}-{}-{case}.db",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let conn = Connection::open(&path)?;
        crate::migrate::run_migrations(&conn)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
        load_vec_extension(&conn)?;
        insert_test_memory(&conn, 1)?;

        // The channel pair is a bounded barrier: old selection/preparation must
        // finish before the foreground commits, and old commit waits for release.
        // Preparation runs on the thread owning the test-only provider env lock;
        // the worker still selects and commits through its independent connection.
        let (selected_tx, selected_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel::<Vec<PreparedMemoryEmbedding>>(1);
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || -> Result<usize> {
            let conn = Connection::open(worker_path)?;
            conn.execute_batch("PRAGMA foreign_keys = ON;")?;
            load_vec_extension(&conn)?;
            let selected = select_memory_embedding_reindex_candidates(&conn, &target(), 1)?;
            assert_eq!(selected.len(), 1);
            assert!(conn.is_autocommit());
            selected_tx.send(selected)?;
            let prepared = release_rx.recv_timeout(Duration::from_secs(10))?;
            upsert_prepared_memory_embedding_batch(&conn, &prepared, &mut vec![])
        });
        let selected = selected_rx.recv_timeout(Duration::from_secs(10))?;
        assert!(conn.is_autocommit(), "model preparation must be outside a transaction");
        let prepared = prepare_memory_embedding_batch(
            &selected,
            &mut vec![],
            &mut EmbeddingFallbackCache::default(),
        )?;
        assert!(conn.is_autocommit());
        let foreground = (|| -> Result<_> {
            conn.execute(mutation, [])?;
            if write_new_vector {
                upsert_memory_embedding_for_row(&conn, 1)?;
                ensure_vec_index(&conn)?;
            }
            stored_embedding(&conn)
        })();
        release_tx.send(prepared)?;
        let processed = worker.join().expect("backfill worker must not panic")?;
        let expected = foreground?;
        assert_eq!(processed, 0, "stale calculation must be discarded: {case}");
        assert_eq!(stored_embedding(&conn)?, expected, "{case}");
        assert_eq!(pending_memory_embedding_reindex_count_for_target(&conn, &target())?, expected_pending, "{case}");
        let coverage = active_embedding_coverage_for_target(&conn, &target())?;
        assert_eq!(coverage.embedded, i64::from(write_new_vector), "{case}");
        if let Some((blob, _, _)) = expected {
            let values = decode_embedding(&blob, EMBEDDING_DIMENSIONS as i64)?;
            let outcome = vector_search_filtered(&conn, &values, VectorSearchFilters::default(), 1)?;
            assert_eq!(outcome.hits[0].memory_id, 1);
            assert!(outcome.hits[0].distance < 0.00001, "derived mirror changed: {case}");
        }
        drop(conn);
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn reindex_repairs_same_second_and_legacy_hash_mismatches_within_limit() -> Result<()> {
    let conn = setup_vector_conn()?;
    let old_hash = crate::retrieval::embedding::memory_index_hash(
        "Credential store",
        "old MySQL passage",
        "architecture",
        None,
        "",
    );
    for (id, hash, timestamp) in [
        (1, old_hash.as_str(), 1),
        (2, "", 1),
        (3, "legacy-hash", i64::MAX),
    ] {
        insert_test_memory(&conn, id)?;
        upsert_embedding_with_metadata(
            &conn,
            id,
            DEFAULT_EMBEDDING_MODEL,
            hash,
            &vec![0.0; EMBEDDING_DIMENSIONS],
            timestamp,
        )?;
    }
    insert_test_memory(&conn, 4)?;
    upsert_memory_embedding_for_row(&conn, 4)?;
    conn.execute(
        "UPDATE memory_embeddings SET updated_at_epoch = 0 WHERE memory_id = 4",
        [],
    )?;
    assert_eq!(
        pending_memory_embedding_reindex_count_for_target(&conn, &target())?,
        3
    );
    assert_eq!(
        active_embedding_coverage_for_target(&conn, &target())?.embedded,
        1
    );
    let first = reindex_memory_embeddings_with_report(&conn, 2)?;
    assert_eq!((first.selected, first.processed), (2, 2));
    assert_eq!(
        pending_memory_embedding_reindex_count_for_target(&conn, &target())?,
        1
    );
    assert_eq!(
        active_embedding_coverage_for_target(&conn, &target())?.embedded,
        3
    );
    let second = reindex_memory_embeddings_with_report(&conn, 2)?;
    assert_eq!((second.selected, second.processed), (1, 1));
    assert_eq!(
        pending_memory_embedding_reindex_count_for_target(&conn, &target())?,
        0
    );
    assert_eq!(
        active_embedding_coverage_for_target(&conn, &target())?.embedded,
        4
    );
    assert_eq!(reindex_memory_embeddings(&conn, 2)?, 0);
    Ok(())
}

#[test]
fn reindex_skipped_full_batch_does_not_strand_later_memories() -> Result<()> {
    let conn = setup_vector_conn()?;
    for id in 1..=(EMBEDDING_REINDEX_WRITE_BATCH_SIZE as i64 + 2) {
        insert_test_memory(&conn, id)?;
    }
    // Simulate an entire selected batch becoming ineligible at the write
    // boundary. A zero processed count is not an empty selection.
    conn.execute_batch(
        "CREATE TRIGGER quarantine_reindex_test BEFORE INSERT ON memory_embeddings
         WHEN NEW.memory_id > 2 BEGIN
             UPDATE memories SET status = 'quarantined' WHERE id = NEW.memory_id;
             SELECT RAISE(IGNORE);
         END;",
    )?;
    assert_eq!(
        reindex_memory_embeddings(&conn, EMBEDDING_REINDEX_WRITE_BATCH_SIZE as i64 + 2)?,
        2
    );
    assert_eq!(
        pending_memory_embedding_reindex_count_for_target(&conn, &target())?,
        0
    );
    assert_eq!(
        active_embedding_coverage_for_target(&conn, &target())?.embedded,
        2
    );
    Ok(())
}

#[test]
fn same_second_enrichment_change_invalidates_coverage_and_selection() -> Result<()> {
    let conn = setup_vector_conn()?;
    insert_test_memory(&conn, 1)?;
    upsert_memory_embedding_for_row(&conn, 1)?;
    conn.execute("UPDATE memory_embeddings SET updated_at_epoch = 1", [])?;
    conn.execute(
        "UPDATE memories SET search_context = 'keywords: newly-enriched-passage',
             search_context_source_hash = 'enrichment-revision-2',
             updated_at_epoch = 1 WHERE id = 1",
        [],
    )?;
    assert_eq!(
        embedding_count(&conn)?,
        1,
        "enrichment update leaves an old vector to detect"
    );
    assert_eq!(
        pending_memory_embedding_reindex_count_for_target(&conn, &target())?,
        1
    );
    assert_eq!(
        active_embedding_coverage_for_target(&conn, &target())?.embedded,
        0
    );
    let selected = select_memory_embedding_reindex_candidates(&conn, &target(), 1)?;
    assert_eq!(selected.len(), 1);
    assert_eq!(
        selected[0].search_context,
        "keywords: newly-enriched-passage"
    );
    assert_eq!(reindex_memory_embeddings(&conn, 1)?, 1);
    assert_eq!(
        pending_memory_embedding_reindex_count_for_target(&conn, &target())?,
        0
    );
    Ok(())
}
