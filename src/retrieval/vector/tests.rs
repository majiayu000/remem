use anyhow::Result;
use rusqlite::Connection;

use super::*;

struct ScopedEmbeddingProvider {
    _guard: std::sync::MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<String>)>,
}

impl ScopedEmbeddingProvider {
    fn new(provider: &str) -> Self {
        let guard = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("env lock should acquire");
        let saved = ENV_KEYS
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect::<Vec<_>>();
        for key in ENV_KEYS {
            unsafe { std::env::remove_var(key) };
        }
        unsafe { std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", provider) };
        Self {
            _guard: guard,
            saved,
        }
    }

    fn api_fallback_off(base_url: &str) -> Self {
        let guard = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("env lock should acquire");
        let saved = ENV_KEYS
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect::<Vec<_>>();
        for key in ENV_KEYS {
            unsafe { std::env::remove_var(key) };
        }
        unsafe {
            std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", "api");
            std::env::set_var("REMEM_EMBEDDINGS_FALLBACK", "off");
            std::env::set_var("REMEM_EMBEDDINGS_API_KEY", "test-key");
            std::env::set_var("REMEM_EMBEDDINGS_BASE_URL", base_url);
        }
        Self {
            _guard: guard,
            saved,
        }
    }
}

impl Drop for ScopedEmbeddingProvider {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..) {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}

struct VectorTestConn {
    conn: Connection,
    _provider: ScopedEmbeddingProvider,
}

impl std::ops::Deref for VectorTestConn {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        &self.conn
    }
}

fn setup_vector_conn_with_provider(provider_name: &str) -> Result<VectorTestConn> {
    let provider = ScopedEmbeddingProvider::new(provider_name);
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(VectorTestConn {
        conn,
        _provider: provider,
    })
}

fn setup_vector_conn() -> Result<VectorTestConn> {
    setup_vector_conn_with_provider("feature-hash")
}

const ENV_KEYS: &[&str] = &[
    "REMEM_CONFIG",
    "REMEM_EMBEDDINGS_PROVIDER",
    "REMEM_EMBEDDING_PROVIDER",
    "REMEM_EMBEDDINGS_FALLBACK",
    "REMEM_EMBEDDINGS_BASE_URL",
    "REMEM_EMBEDDING_BASE_URL",
    "REMEM_EMBEDDINGS_API_KEY",
    "REMEM_EMBEDDING_API_KEY",
    "OPENAI_API_KEY",
];

fn with_embedding_provider<T>(provider: &str, f: impl FnOnce() -> T) -> T {
    let _provider = ScopedEmbeddingProvider::new(provider);
    f()
}

fn insert_test_memory(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
         VALUES (?1, '/repo', 'Credential store', 'SQLCipher encrypts secrets at rest.', 'architecture', 1, 1, 'active')",
        params![id],
    )?;
    Ok(())
}

#[test]
fn off_provider_skips_vector_writes_backfill_and_search() -> Result<()> {
    let conn = setup_vector_conn_with_provider("off")?;
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
    assert_eq!(pending_memory_embedding_reindex_count(&conn)?, 0);

    let report = reindex_memory_embeddings_with_report(&conn, 100)?;
    assert_eq!(report.processed, 0);
    assert_eq!(report.model, "off");

    let query = vec![0.0; EMBEDDING_DIMENSIONS];
    let outcome = vector_search_filtered(&conn, &query, VectorSearchFilters::default(), 10)?;
    assert_eq!(
        outcome.disabled_reason.as_deref(),
        Some("embedding provider is off")
    );
    assert_eq!(embedding_count(&conn)?, 0);
    Ok(())
}

#[test]
fn api_failure_fallback_off_skips_vector_writes_and_backfill() -> Result<()> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let handle = std::thread::spawn(move || -> Result<()> {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept()?;
            let mut buffer = [0_u8; 8192];
            std::io::Read::read(&mut stream, &mut buffer)?;
            let body = "provider unavailable";
            let response = format!(
                "HTTP/1.1 500 Internal Server Error\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            std::io::Write::write_all(&mut stream, response.as_bytes())?;
        }
        Ok(())
    });
    let _provider = ScopedEmbeddingProvider::api_fallback_off(&format!("http://{addr}/v1"));
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
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

    let report = reindex_memory_embeddings_with_report(&conn, 100)?;
    handle
        .join()
        .map_err(|_| anyhow::anyhow!("embedding test server thread panicked"))??;
    assert_eq!(report.processed, 0);
    assert_eq!(report.model, "off");
    assert_eq!(embedding_count(&conn)?, 0);
    Ok(())
}

#[test]
fn vector_search_returns_nearest_memory_embedding() -> Result<()> {
    let conn = setup_vector_conn()?;
    conn.execute(
        "INSERT INTO memories
         (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
         VALUES
         (1, '/repo', 'Credential store', 'SQLCipher encrypts secrets at rest.', 'architecture', 1, 1, 'active'),
         (2, '/repo', 'Posting workflow', 'Publish social media drafts after review.', 'procedure', 1, 1, 'active')",
        [],
    )?;
    upsert_memory_embedding(
        &conn,
        1,
        "Credential store",
        "SQLCipher encrypts secrets at rest.",
        "architecture",
        None,
    )?;
    upsert_memory_embedding(
        &conn,
        2,
        "Posting workflow",
        "Publish social media drafts after review.",
        "procedure",
        None,
    )?;

    let query = embed_query_text("How do we protect private persisted data?");
    let outcome = vector_search_filtered(
        &conn,
        &query,
        VectorSearchFilters {
            project: Some("/repo"),
            ..VectorSearchFilters::default()
        },
        5,
    )?;

    assert!(outcome.disabled_reason.is_none());
    assert_eq!(outcome.hits[0].memory_id, 1);
    Ok(())
}

#[test]
fn vector_search_respects_filters() -> Result<()> {
    let conn = setup_vector_conn()?;
    for (id, project, branch, memory_type, status) in [
        (1, "/repo", Some("main"), "architecture", "active"),
        (2, "/other", Some("main"), "architecture", "active"),
        (3, "/repo", Some("feature"), "architecture", "active"),
        (4, "/repo", Some("main"), "decision", "active"),
        (5, "/repo", Some("main"), "architecture", "stale"),
    ] {
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status, branch)
             VALUES (?1, ?2, 'Credential store', 'SQLCipher encrypts secrets at rest.', ?3, 1, 1, ?4, ?5)",
            params![id, project, memory_type, status, branch],
        )?;
        upsert_memory_embedding(
            &conn,
            id,
            "Credential store",
            "SQLCipher encrypts secrets at rest.",
            memory_type,
            None,
        )?;
    }

    let query = embed_query_text("protect private persisted data");
    let outcome = vector_search_filtered(
        &conn,
        &query,
        VectorSearchFilters {
            project: Some("/repo"),
            branch: Some("main"),
            memory_type: Some("architecture"),
            include_stale: false,
        },
        10,
    )?;
    let ids: Vec<i64> = outcome.hits.iter().map(|hit| hit.memory_id).collect();

    assert_eq!(ids, vec![1]);
    Ok(())
}

#[test]
fn vector_search_uses_profile_memory_id_index_for_embedding_fetch() -> Result<()> {
    let conn = setup_vector_conn()?;
    insert_test_memory(&conn, 1)?;
    upsert_memory_embedding(
        &conn,
        1,
        "Credential store",
        "SQLCipher encrypts secrets at rest.",
        "architecture",
        None,
    )?;

    let plan = conn
        .prepare(
            "EXPLAIN QUERY PLAN
             SELECT memory_id, embedding, dimensions
             FROM memory_embeddings INDEXED BY idx_memory_embeddings_profile_memory_id
             WHERE model = ?1 AND dimensions = ?2 AND memory_id IN (?3)",
        )?
        .query_map(
            params![DEFAULT_EMBEDDING_MODEL, EMBEDDING_DIMENSIONS as i64, 1_i64],
            |row| row.get::<_, String>(3),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    assert!(
        plan.iter()
            .any(|detail| detail.contains("idx_memory_embeddings_profile_memory_id")),
        "embedding fetch should use profile memory_id index, got {plan:#?}"
    );
    Ok(())
}

#[test]
fn explicit_embedding_backfill_covers_all_statuses_across_batches() -> Result<()> {
    let conn = setup_vector_conn()?;
    for id in 1..=1_002 {
        let status = match id {
            1 => "stale",
            2 => "archived",
            _ => "active",
        };
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES (?1, '/repo', 'Backfill memory', 'Backfill should cover all visible statuses.', 'decision', 1, ?1, ?2)",
            params![id, status],
        )?;
    }

    ensure_vec_table(&conn)?;
    assert_eq!(backfill_missing_memory_embeddings(&conn, 1_000)?, 1_000);
    assert_eq!(backfill_missing_memory_embeddings(&conn, 1_000)?, 2);

    let count: i64 = conn.query_row("SELECT COUNT(*) FROM memory_embeddings", [], |row| {
        row.get(0)
    })?;
    assert_eq!(count, 1_002);
    for status in ["stale", "archived"] {
        let status_count: i64 = conn.query_row(
            "SELECT COUNT(*)
             FROM memory_embeddings e
             JOIN memories m ON m.id = e.memory_id
             WHERE m.status = ?1",
            [status],
            |row| row.get(0),
        )?;
        assert_eq!(status_count, 1);
    }
    Ok(())
}

#[test]
fn reindex_report_includes_profile_timings_and_remaining_work() -> Result<()> {
    let conn = setup_vector_conn()?;
    for id in 1..=3 {
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES (?1, '/repo', 'Backfill memory', 'Measured backfill report.', 'decision', 1, ?1, 'active')",
            params![id],
        )?;
    }

    ensure_vec_table(&conn)?;
    let report = reindex_memory_embeddings_with_report(&conn, 2)?;

    assert_eq!(report.selected, 2);
    assert_eq!(report.processed, 2);
    assert_eq!(report.model, DEFAULT_EMBEDDING_MODEL);
    assert_eq!(report.dimensions, EMBEDDING_DIMENSIONS);
    assert_eq!(pending_memory_embedding_reindex_count(&conn)?, 1);

    let phases: Vec<&str> = report
        .timings
        .iter()
        .map(|timing| timing.phase.as_str())
        .collect();
    for expected in [
        "profile_probe",
        "select_pending",
        "embed_memory",
        "upsert_embeddings",
        "commit",
        "total",
    ] {
        assert!(
            phases.contains(&expected),
            "missing timing phase {expected}; got {phases:?}"
        );
    }
    Ok(())
}

#[test]
fn reindex_batch_rolls_back_failed_upserts() -> Result<()> {
    let conn = setup_vector_conn()?;
    for (id, updated_at_epoch) in [(1_i64, 2_i64), (2, 1)] {
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES (?1, '/repo', 'Backfill memory', 'Batch rollback should be atomic.', 'decision', 1, ?2, 'active')",
            params![id, updated_at_epoch],
        )?;
    }
    conn.execute_batch(
        "CREATE TRIGGER fail_embedding_for_memory_2
         BEFORE INSERT ON memory_embeddings
         WHEN NEW.memory_id = 2
         BEGIN
             SELECT RAISE(FAIL, 'forced embedding failure');
         END;",
    )?;

    let error = reindex_memory_embeddings_with_report(&conn, 2).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("memory id=2"), "{message}");
    assert!(message.contains("forced embedding failure"), "{message}");
    assert_eq!(embedding_count(&conn)?, 0);
    assert_eq!(pending_memory_embedding_reindex_count(&conn)?, 2);
    Ok(())
}

#[test]
fn vector_search_ignores_embeddings_from_other_models() -> Result<()> {
    let conn = setup_vector_conn()?;
    insert_test_memory(&conn, 1)?;
    upsert_memory_embedding(
        &conn,
        1,
        "Credential store",
        "SQLCipher encrypts secrets at rest.",
        "architecture",
        None,
    )?;

    let query = TextEmbedding::new("remote-test-model", vec![0.1, 0.2, 0.3])?;
    let outcome = vector_search_embedding_filtered(
        &conn,
        &query,
        VectorSearchFilters {
            project: Some("/repo"),
            ..VectorSearchFilters::default()
        },
        5,
    )?;

    assert!(outcome.hits.is_empty());
    assert!(outcome
        .disabled_reason
        .as_deref()
        .unwrap_or("")
        .contains("remote-test-model"));
    Ok(())
}

#[test]
fn backfill_rebuilds_embeddings_from_stale_model() -> Result<()> {
    let conn = setup_vector_conn()?;
    insert_test_memory(&conn, 1)?;
    let stale_blob = vec![0u8; 3 * std::mem::size_of::<f32>()];
    conn.execute(
        "INSERT INTO memory_embeddings
         (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
         VALUES (1, ?1, 3, 'old-model', 'old-hash', 1)",
        params![stale_blob],
    )?;

    assert_eq!(pending_memory_embedding_count(&conn)?, 1);
    assert_eq!(backfill_missing_memory_embeddings(&conn, 100)?, 1);

    let row: (String, i64) = conn.query_row(
        "SELECT model, dimensions FROM memory_embeddings WHERE memory_id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(row.0, DEFAULT_EMBEDDING_MODEL);
    assert_eq!(row.1, EMBEDDING_DIMENSIONS as i64);
    assert_eq!(pending_memory_embedding_count(&conn)?, 0);
    Ok(())
}

#[test]
fn reindex_rebuilds_embeddings_when_memory_is_newer_than_embedding() -> Result<()> {
    let conn = setup_vector_conn()?;
    insert_test_memory(&conn, 1)?;
    upsert_memory_embedding(
        &conn,
        1,
        "Credential store",
        "SQLCipher encrypts secrets at rest.",
        "architecture",
        None,
    )?;
    let before_hash: String = conn.query_row(
        "SELECT content_hash FROM memory_embeddings WHERE memory_id = 1",
        [],
        |row| row.get(0),
    )?;
    conn.execute(
        "UPDATE memory_embeddings SET updated_at_epoch = 1 WHERE memory_id = 1",
        [],
    )?;
    conn.execute(
        "UPDATE memories
         SET content = 'SQLCipher protects the local database with encryption at rest.',
             updated_at_epoch = ?1
         WHERE id = 1",
        params![2],
    )?;

    assert_eq!(pending_memory_embedding_reindex_count(&conn)?, 1);
    assert_eq!(reindex_memory_embeddings(&conn, 100)?, 1);

    let after_hash: String = conn.query_row(
        "SELECT content_hash FROM memory_embeddings WHERE memory_id = 1",
        [],
        |row| row.get(0),
    )?;
    assert_ne!(after_hash, before_hash);
    assert_eq!(pending_memory_embedding_reindex_count(&conn)?, 0);
    Ok(())
}

#[test]
fn empty_vector_table_with_memories_is_reported_as_disabled() -> Result<()> {
    let conn = setup_vector_conn()?;
    conn.execute(
        "INSERT INTO memories
         (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
         VALUES (1, '/repo', 'Needs embedding', 'Backfill should be explicit.', 'decision', 1, 1, 'active')",
        [],
    )?;
    let query = embed_query_text("needs embedding");

    let outcome = vector_search_filtered(&conn, &query, VectorSearchFilters::default(), 10)?;

    assert!(outcome.hits.is_empty());
    assert!(outcome
        .disabled_reason
        .as_deref()
        .unwrap_or("")
        .contains("reindex-embeddings"));
    Ok(())
}

#[test]
fn missing_vector_table_is_reported_as_disabled() -> Result<()> {
    with_embedding_provider("feature-hash", || -> Result<()> {
        let conn = Connection::open_in_memory()?;
        let query = embed_query_text("anything");
        let outcome = vector_search_filtered(&conn, &query, VectorSearchFilters::default(), 10)?;

        assert!(outcome
            .disabled_reason
            .as_deref()
            .unwrap_or("")
            .contains("memory_embeddings table is missing"));
        assert!(outcome.hits.is_empty());
        Ok(())
    })
}
