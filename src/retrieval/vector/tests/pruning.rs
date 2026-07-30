#[cfg(feature = "local-onnx")]
use std::path::{Path, PathBuf};
#[cfg(all(feature = "local-onnx", not(windows)))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(all(feature = "local-onnx", not(windows)))]
use std::time::{Duration, Instant};

#[cfg(feature = "local-onnx")]
use anyhow::Context;
use rusqlite::params;

use super::*;
use crate::retrieval::embedding::EmbeddingBackfillTarget;

#[cfg(all(feature = "local-onnx", not(windows)))]
static PRUNE_DB_BUSY: AtomicBool = AtomicBool::new(false);

#[cfg(all(feature = "local-onnx", not(windows)))]
fn signal_prune_db_busy(_attempt: i32) -> bool {
    PRUNE_DB_BUSY.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(1));
    true
}

#[cfg(feature = "local-onnx")]
struct PruneRaceDir(PathBuf);

#[cfg(feature = "local-onnx")]
impl PruneRaceDir {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "remem-prune-model-race-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&path)
            .with_context(|| format!("create prune race fixture {}", path.display()))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

#[cfg(feature = "local-onnx")]
impl Drop for PruneRaceDir {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!(
                "failed to remove prune race fixture {}: {error}",
                self.0.display()
            );
        }
    }
}

#[test]
#[cfg(feature = "local-onnx")]
fn auto_provider_runtime_unavailable_defers_passage_write() -> Result<()> {
    let conn = setup_vector_conn_with_provider("auto")?;
    let fixture = PruneRaceDir::new()?;
    let model_root = fixture.path().join("models");
    crate::retrieval::embedding::install_test_local_embedding_model(&model_root)?;
    unsafe { std::env::set_var("REMEM_EMBEDDINGS_MODEL_DIR", &model_root) };
    insert_test_memory(&conn, 1)?;
    ensure_vec_table(&conn)?;
    let _failure = crate::retrieval::embedding::fail_next_test_model_embed_unavailable(
        &model_root,
        "synthetic passage model race",
    )?;

    upsert_memory_embedding(
        &conn,
        1,
        "Credential store",
        "SQLCipher encrypts secrets at rest.",
        "architecture",
        None,
        "",
    )?;

    assert_eq!(embedding_count(&conn)?, 0);
    Ok(())
}

#[test]
#[cfg(feature = "local-onnx")]
fn auto_provider_runtime_unavailable_rejects_generic_backfill_probe() -> Result<()> {
    let _provider = ScopedEmbeddingProvider::new("auto");
    let fixture = PruneRaceDir::new()?;
    let model_root = fixture.path().join("models");
    crate::retrieval::embedding::install_test_local_embedding_model(&model_root)?;
    unsafe { std::env::set_var("REMEM_EMBEDDINGS_MODEL_DIR", &model_root) };
    let _failure = crate::retrieval::embedding::fail_next_test_model_embed_unavailable(
        &model_root,
        "synthetic generic backfill model race",
    )?;

    let error = EmbeddingBackfillSession::start()
        .expect_err("Generic profile probe must preserve typed local-unavailable");

    assert!(
        crate::retrieval::embedding::is_local_embedding_model_unavailable_error(&error),
        "{error:#}"
    );
    Ok(())
}

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
        "",
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
        "",
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
        "",
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

#[test]
fn prune_inactive_profiles_requires_fresh_active_rows() -> Result<()> {
    let conn = setup_vector_conn()?;
    for id in 1..=2 {
        insert_test_memory(&conn, id)?;
    }
    ensure_vec_table(&conn)?;
    let active_blob = vec![0u8; EMBEDDING_DIMENSIONS * std::mem::size_of::<f32>()];
    let old_blob = vec![0u8; 3 * std::mem::size_of::<f32>()];
    for id in 1..=2 {
        conn.execute(
            "INSERT INTO memory_embeddings
             (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
             VALUES (?1, ?2, ?3, ?4, 'active-old-hash', 1)",
            params![
                id,
                &active_blob,
                EMBEDDING_DIMENSIONS as i64,
                DEFAULT_EMBEDDING_MODEL
            ],
        )?;
        conn.execute(
            "INSERT INTO memory_embeddings
             (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
             VALUES (?1, ?2, 3, 'old-model', 'old-hash', 1)",
            params![id, &old_blob],
        )?;
    }
    conn.execute("UPDATE memories SET updated_at_epoch = 10 WHERE id = 1", [])?;
    let target = EmbeddingBackfillTarget {
        model: DEFAULT_EMBEDDING_MODEL.to_string(),
        dimensions: EMBEDDING_DIMENSIONS,
    };

    let error = prune_inactive_memory_embeddings(&conn, &target).unwrap_err();

    assert!(
        error.to_string().contains("missing or stale rows"),
        "{error:#}"
    );
    let old_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_embeddings WHERE model = 'old-model'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(old_rows, 2);
    Ok(())
}

#[test]
fn prune_rejects_target_that_is_not_currently_active() -> Result<()> {
    let conn = setup_vector_conn()?;
    insert_test_memory(&conn, 1)?;
    ensure_vec_table(&conn)?;
    let active_blob = vec![0u8; EMBEDDING_DIMENSIONS * std::mem::size_of::<f32>()];
    let old_blob = vec![0u8; 3 * std::mem::size_of::<f32>()];
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
         VALUES (1, ?1, 3, 'old-model', 'old-hash', 1)",
        params![&old_blob],
    )?;
    let stale_target = EmbeddingBackfillTarget {
        model: "old-model".to_string(),
        dimensions: 3,
    };

    let error = prune_inactive_memory_embeddings(&conn, &stale_target)
        .expect_err("prune must re-resolve and preserve the current profile");
    let message = format!("{error:#}");

    assert!(message.contains("current embedding profile"), "{message}");
    let active_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_embeddings WHERE model = ?1 AND dimensions = ?2",
        params![DEFAULT_EMBEDDING_MODEL, EMBEDDING_DIMENSIONS as i64],
        |row| row.get(0),
    )?;
    assert_eq!(active_rows, 1);
    Ok(())
}

#[test]
#[cfg(all(feature = "local-onnx", not(windows)))]
fn prune_holds_model_state_pin_until_delete_commits() -> Result<()> {
    let fixture = PruneRaceDir::new()?;
    let db_path = fixture.path().join("prune.sqlite");
    let model_root = fixture.path().join("models");
    let install_dir = model_root.join("fastembed-intfloat-multilingual-e5-small-v1");
    let setup = Connection::open(&db_path)?;
    crate::migrate::run_migrations(&setup)?;
    insert_test_memory(&setup, 1)?;
    ensure_vec_table(&setup)?;
    let feature_hash_blob = vec![0u8; EMBEDDING_DIMENSIONS * std::mem::size_of::<f32>()];
    let dormant_local_blob = vec![0u8; 384 * std::mem::size_of::<f32>()];
    setup.execute(
        "INSERT INTO memory_embeddings
         (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
         VALUES (1, ?1, ?2, ?3, 'feature-hash', 1)",
        params![
            &feature_hash_blob,
            EMBEDDING_DIMENSIONS as i64,
            DEFAULT_EMBEDDING_MODEL
        ],
    )?;
    setup.execute(
        "INSERT INTO memory_embeddings
         (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
         VALUES (1, ?1, 384, 'previous-local-artifact', 'local', 1)",
        params![&dormant_local_blob],
    )?;
    drop(setup);

    PRUNE_DB_BUSY.store(false, Ordering::SeqCst);
    let prune_db_path = db_path.clone();
    let prune_model_root = model_root.clone();
    let target = EmbeddingBackfillTarget {
        model: DEFAULT_EMBEDDING_MODEL.to_string(),
        dimensions: EMBEDDING_DIMENSIONS,
    };
    let (prune_ready_tx, prune_ready_rx) = std::sync::mpsc::sync_channel(1);
    let (prune_start_tx, prune_start_rx) = std::sync::mpsc::sync_channel(1);
    let prune = std::thread::spawn(move || -> Result<InactiveEmbeddingPruneReport> {
        let _provider = ScopedEmbeddingProvider::new("auto");
        unsafe {
            std::env::set_var("REMEM_EMBEDDINGS_MODEL_DIR", &prune_model_root);
        }
        prune_ready_tx.send(())?;
        prune_start_rx.recv()?;
        let conn = Connection::open(prune_db_path)?;
        conn.busy_handler(Some(signal_prune_db_busy))?;
        prune_inactive_memory_embeddings(&conn, &target)
    });

    // Full-suite peers may hold TEST_ENV_LOCK for several seconds. Start the
    // database-contention deadline only after this worker owns that lock and
    // has installed its environment.
    prune_ready_rx
        .recv_timeout(Duration::from_secs(30))
        .context("prune worker did not acquire the test environment lock")?;
    let blocker = Connection::open(&db_path)?;
    blocker.execute_batch("BEGIN IMMEDIATE")?;
    assert!(
        !blocker.is_autocommit(),
        "blocker must hold an explicit write transaction"
    );
    assert_eq!(
        blocker.execute(
            "UPDATE memory_embeddings
             SET updated_at_epoch = updated_at_epoch
             WHERE model = 'previous-local-artifact'",
            [],
        )?,
        1,
        "blocker must own an uncommitted write to the row prune will delete"
    );
    prune_start_tx.send(())?;

    let busy_deadline = Instant::now() + Duration::from_secs(3);
    while !PRUNE_DB_BUSY.load(Ordering::SeqCst) && Instant::now() < busy_deadline {
        if prune.is_finished() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if !PRUNE_DB_BUSY.load(Ordering::SeqCst) {
        blocker.execute_batch("ROLLBACK")?;
        let result = prune
            .join()
            .map_err(|_| anyhow::anyhow!("prune race worker panicked"))?;
        anyhow::bail!("prune never reached the blocked DELETE: {result:?}");
    }

    let (contended_tx, contended_rx) = std::sync::mpsc::sync_channel(1);
    let (acquired_tx, acquired_rx) = std::sync::mpsc::sync_channel(1);
    let activation = std::thread::spawn(move || -> Result<()> {
        std::fs::create_dir_all(&install_dir)?;
        let state_lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(install_dir.join(".remem-model-state.lock"))?;
        match fs2::FileExt::try_lock_exclusive(&state_lock) {
            Ok(()) => contended_tx.send(false)?,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                contended_tx.send(true)?;
                fs2::FileExt::lock_exclusive(&state_lock)?;
            }
            Err(error) => return Err(error.into()),
        }
        acquired_tx.send(())?;
        Ok(())
    });
    let activation_observed_contention = contended_rx
        .recv_timeout(Duration::from_secs(1))
        .context("activation did not report the model-state lock result")?;

    blocker.execute_batch("ROLLBACK")?;
    let report = prune
        .join()
        .map_err(|_| anyhow::anyhow!("prune race worker panicked"))??;
    acquired_rx
        .recv_timeout(Duration::from_secs(2))
        .context("activation stayed blocked after prune released the state pin")?;
    activation
        .join()
        .map_err(|_| anyhow::anyhow!("activation race worker panicked"))??;

    assert!(
        activation_observed_contention,
        "local activation acquired the model-state lock while prune was blocked before DELETE"
    );
    assert_eq!(report.pruned, 1);
    Ok(())
}
