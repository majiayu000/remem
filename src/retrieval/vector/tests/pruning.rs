use rusqlite::params;

use super::*;
use crate::retrieval::embedding::EmbeddingBackfillTarget;

#[test]
fn local_model_unavailable_defers_memory_embedding_write() -> Result<()> {
    let conn = setup_vector_conn_with_provider("local")?;
    let model_dir = std::env::temp_dir().join(format!(
        "remem-empty-vector-models-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    unsafe { std::env::set_var("REMEM_EMBEDDINGS_MODEL_DIR", &model_dir) };
    insert_test_memory(&conn, 1)?;
    ensure_vec_table(&conn)?;

    upsert_memory_embedding(
        &conn,
        1,
        "Credential store",
        "SQLCipher encrypts secrets at rest.",
        "architecture",
        None,
    )?;

    assert_eq!(embedding_count(&conn)?, 0);
    Ok(())
}

#[test]
fn prune_inactive_profiles_requires_complete_active_coverage() -> Result<()> {
    let conn = setup_vector_conn()?;
    for id in 1..=2 {
        insert_test_memory(&conn, id)?;
    }
    ensure_vec_table(&conn)?;
    let old_blob = vec![0u8; 3 * std::mem::size_of::<f32>()];
    for id in 1..=2 {
        conn.execute(
            "INSERT INTO memory_embeddings
             (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
             VALUES (?1, ?2, 3, 'old-model', 'old-hash', 1)",
            params![id, &old_blob],
        )?;
    }
    let target = EmbeddingBackfillTarget {
        model: DEFAULT_EMBEDDING_MODEL.to_string(),
        dimensions: EMBEDDING_DIMENSIONS,
    };

    upsert_memory_embedding(
        &conn,
        1,
        "Credential store",
        "SQLCipher encrypts secrets at rest.",
        "architecture",
        None,
    )?;
    let error = prune_inactive_memory_embeddings(&conn, &target).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("before active coverage reaches 100%"),
        "{error:#}"
    );

    upsert_memory_embedding(
        &conn,
        2,
        "Credential store",
        "SQLCipher encrypts secrets at rest.",
        "architecture",
        None,
    )?;
    let report = prune_inactive_memory_embeddings(&conn, &target)?;

    assert_eq!(report.pruned, 2);
    assert_eq!(report.coverage.embedded, 2);
    assert_eq!(report.coverage.total, 2);
    let old_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_embeddings WHERE model = 'old-model'",
        [],
        |row| row.get(0),
    )?;
    let active_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_embeddings WHERE model = ?1 AND dimensions = ?2",
        params![DEFAULT_EMBEDDING_MODEL, EMBEDDING_DIMENSIONS as i64],
        |row| row.get(0),
    )?;
    assert_eq!(old_rows, 0);
    assert_eq!(active_rows, 2);
    Ok(())
}
