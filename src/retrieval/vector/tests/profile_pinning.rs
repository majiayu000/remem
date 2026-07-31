use std::io::{Read, Write};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

use anyhow::Result;
use rusqlite::{params, Connection};

use super::*;
use crate::retrieval::embedding::EmbeddingBackfillTarget;

struct ModelSequenceEmbeddingServer {
    base_url: String,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Result<()>>>,
}

impl ModelSequenceEmbeddingServer {
    fn start(models: &[&str]) -> Result<Self> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let models = models
            .iter()
            .map(|model| (*model).to_string())
            .collect::<Vec<_>>();
        let calls = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let calls_for_thread = Arc::clone(&calls);
        let stop_for_thread = Arc::clone(&stop);
        let handle = std::thread::spawn(move || -> Result<()> {
            while !stop_for_thread.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false)?;
                        let call_index = calls_for_thread.fetch_add(1, Ordering::SeqCst);
                        let mut buffer = [0u8; 8192];
                        let _ = stream.read(&mut buffer)?;
                        let model = models
                            .get(call_index)
                            .or_else(|| models.last())
                            .expect("model sequence should not be empty");
                        let body = format!(
                            r#"{{"data":[{{"embedding":[0.1,0.2,0.3,0.4]}}],"model":"{model}"}}"#
                        );
                        let response = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        stream.write_all(response.as_bytes())?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(())
        });
        Ok(Self {
            base_url: format!("http://{addr}/v1"),
            stop,
            handle: Some(handle),
        })
    }
}

impl Drop for ModelSequenceEmbeddingServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .expect("embedding server thread should not panic")
                .expect("embedding server should stop cleanly");
        }
    }
}

#[test]
fn reindex_rejects_intra_batch_profile_drift_without_writes() -> Result<()> {
    let server = ModelSequenceEmbeddingServer::start(&["profile-a", "profile-a", "profile-b"])?;
    let _provider = ScopedEmbeddingProvider::new("api");
    unsafe {
        std::env::set_var("REMEM_EMBEDDINGS_API_KEY", "test-key");
        std::env::set_var("REMEM_EMBEDDINGS_BASE_URL", &server.base_url);
    }
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    for id in 1_i64..=2 {
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES (?1, '/repo', 'Pinned profile', 'Profile drift must fail atomically.', 'decision', 1, ?1, 'active')",
            params![id],
        )?;
    }
    ensure_vec_table(&conn)?;

    let error = reindex_memory_embeddings_with_report(&conn, 2)
        .expect_err("profile drift must abort before any batch write");
    let message = format!("{error:#}");

    assert!(
        message.contains("pinned embedding profile changed"),
        "{message}"
    );
    assert!(message.contains("profile-a"), "{message}");
    assert!(message.contains("profile-b"), "{message}");
    assert_eq!(embedding_count(&conn)?, 0);
    Ok(())
}

#[test]
fn reindex_keeps_one_profile_pinned_across_batches() -> Result<()> {
    let server = ModelSequenceEmbeddingServer::start(&["profile-a", "profile-a", "profile-b"])?;
    let _provider = ScopedEmbeddingProvider::new("api");
    unsafe {
        std::env::set_var("REMEM_EMBEDDINGS_API_KEY", "test-key");
        std::env::set_var("REMEM_EMBEDDINGS_BASE_URL", &server.base_url);
    }
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    for id in 1_i64..=2 {
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES (?1, '/repo', 'Pinned profile', 'Cross-batch drift must fail.', 'decision', 1, ?1, 'active')",
            params![id],
        )?;
    }
    ensure_vec_table(&conn)?;
    let mut session = EmbeddingBackfillSession::start()?;
    assert_eq!(session.target().model, "profile-a");

    let first = reindex_memory_embeddings_with_session_report(&conn, 1, &mut session)?;
    assert_eq!(first.processed, 1);
    assert_eq!(first.model, "profile-a");

    let error = reindex_memory_embeddings_with_session_report(&conn, 1, &mut session)
        .expect_err("the second batch must retain the first batch's profile");
    let message = format!("{error:#}");
    assert!(
        message.contains("pinned embedding profile changed"),
        "{message}"
    );
    assert!(message.contains("profile-a"), "{message}");
    assert!(message.contains("profile-b"), "{message}");

    let profile_a_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_embeddings WHERE model = 'profile-a'",
        [],
        |row| row.get(0),
    )?;
    let profile_b_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_embeddings WHERE model = 'profile-b'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(profile_a_rows, 1);
    assert_eq!(profile_b_rows, 0);
    Ok(())
}

#[test]
fn prune_preconditions_reject_initial_typed_fallback() -> Result<()> {
    let server = FailingEmbeddingServer::start()?;
    let _provider = ScopedEmbeddingProvider::api_fallback_feature_hash(&server.base_url);
    let mut session = EmbeddingBackfillSession::start()?;

    assert_eq!(session.target().model, DEFAULT_EMBEDDING_MODEL);
    let error = session
        .ensure_prune_preconditions()
        .expect_err("a fallback-selected target must never authorize prune");
    let message = format!("{error:#}");

    assert!(message.contains("unsafe provider transition"), "{message}");
    assert!(message.contains("typed provider fallback"), "{message}");
    Ok(())
}

#[test]
fn prune_preconditions_reject_config_change_even_when_target_is_unchanged() -> Result<()> {
    let _provider = ScopedEmbeddingProvider::new("feature-hash");
    let mut session = EmbeddingBackfillSession::start()?;
    assert_eq!(session.target().model, DEFAULT_EMBEDDING_MODEL);

    unsafe { std::env::set_var("REMEM_EMBEDDINGS_TIMEOUT_SECS", "31") };
    let error = session
        .ensure_prune_preconditions()
        .expect_err("any configuration change must invalidate prune authorization");
    let message = format!("{error:#}");

    assert!(
        message.contains("embedding configuration changed"),
        "{message}"
    );
    Ok(())
}

#[test]
fn direct_reindex_does_not_silently_treat_unavailable_fallback_off_as_disabled() -> Result<()> {
    let _provider = ScopedEmbeddingProvider::new("api");
    unsafe { std::env::set_var("REMEM_EMBEDDINGS_FALLBACK", "off") };
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    insert_test_memory(&conn, 1)?;
    ensure_vec_table(&conn)?;

    let error = reindex_memory_embeddings(&conn, 1)
        .expect_err("provider failure with fallback=off must remain visible");
    let message = format!("{error:#}");

    assert!(
        message.contains("requires REMEM_EMBEDDINGS_API_KEY"),
        "{message}"
    );
    assert!(
        message.contains("fallback off disabled provider fallback"),
        "{message}"
    );
    assert_eq!(embedding_count(&conn)?, 0);
    Ok(())
}

#[test]
fn pinned_pending_and_coverage_ignore_other_profiles() -> Result<()> {
    let conn = setup_vector_conn()?;
    for id in 1_i64..=2 {
        insert_test_memory(&conn, id)?;
    }
    ensure_vec_table(&conn)?;
    let target = EmbeddingBackfillTarget {
        model: DEFAULT_EMBEDDING_MODEL.to_string(),
        dimensions: EMBEDDING_DIMENSIONS,
    };
    let active_blob = vec![0u8; EMBEDDING_DIMENSIONS * std::mem::size_of::<f32>()];
    let other_blob = vec![0u8; 3 * std::mem::size_of::<f32>()];
    conn.execute(
        "INSERT INTO memory_embeddings
         (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
         VALUES (1, ?1, ?2, ?3, 'active-hash', 1)",
        params![
            &active_blob,
            EMBEDDING_DIMENSIONS as i64,
            DEFAULT_EMBEDDING_MODEL
        ],
    )?;
    conn.execute(
        "INSERT INTO memory_embeddings
         (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
         VALUES (2, ?1, 3, 'other-profile', 'other-hash', 1)",
        params![&other_blob],
    )?;

    let pending = pending_memory_embedding_reindex_count_for_target(&conn, &target)?;
    let coverage = active_embedding_coverage_for_target(&conn, &target)?;

    assert_eq!(pending, 1);
    assert_eq!(coverage.embedded, 1);
    assert_eq!(coverage.total, 2);
    assert_eq!(coverage.percent, 50.0);
    assert_eq!(coverage.mixed_profile_count, 2);
    Ok(())
}
