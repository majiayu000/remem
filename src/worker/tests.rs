use rusqlite::params;

use crate::db::{self, test_support::ScopedTestDataDir};

use super::{lock, run};
use test_support::install_stub_codex;

mod test_support;

#[tokio::test]
async fn worker_skips_legacy_observation_job_without_retry() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-skip-legacy-observation");
    let conn = db::open_db()?;
    let job_id = db::enqueue_job(
        &conn,
        "codex-cli",
        db::JobType::Observation,
        "/tmp/remem",
        Some("sess-legacy-observation"),
        r#"{"host":"codex-cli","session_id":"sess-legacy-observation","project":"/tmp/remem"}"#,
        50,
    )?;

    run(true, 10).await?;

    let conn = db::open_db()?;
    let (state, attempt_count, last_error): (String, i64, Option<String>) = conn.query_row(
        "SELECT state, attempt_count, last_error FROM jobs WHERE id = ?1",
        params![job_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    anyhow::ensure!(
        state == "done",
        "expected skipped legacy job done, got {state}"
    );
    anyhow::ensure!(
        attempt_count == 0,
        "legacy job should not retry, got {attempt_count}"
    );
    anyhow::ensure!(
        last_error.is_none(),
        "legacy job should not record an error"
    );
    Ok(())
}

#[tokio::test]
async fn worker_rejects_legacy_summary_job_without_retry() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-reject-legacy-summary");
    let conn = db::open_db()?;
    let job_id = db::enqueue_job(
        &conn,
        "codex-cli",
        db::JobType::Summary,
        "/tmp/remem",
        Some("sess-legacy-summary"),
        r#"{"host":"codex-cli","session_id":"sess-legacy-summary","project":"/tmp/remem"}"#,
        50,
    )?;
    drop(conn);

    run(true, 10).await?;

    let conn = db::open_db()?;
    let (state, attempt_count, next_retry, last_error, failure_class): (
        String,
        i64,
        i64,
        Option<String>,
        Option<String>,
    ) = conn.query_row(
        "SELECT state, attempt_count, next_retry_epoch, last_error, failure_class
             FROM jobs WHERE id = ?1",
        params![job_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    anyhow::ensure!(state == "failed", "expected failed job, got {state}");
    anyhow::ensure!(
        attempt_count == 1,
        "legacy Summary rejection should consume one attempt, got {attempt_count}"
    );
    anyhow::ensure!(next_retry == 0, "legacy Summary job should not retry");
    anyhow::ensure!(
        last_error
            .as_deref()
            .is_some_and(|err| err.contains("legacy Summary jobs are retired")),
        "expected retired Summary error, got {last_error:?}"
    );
    anyhow::ensure!(
        failure_class.as_deref() == Some("permanent"),
        "expected permanent failure, got {failure_class:?}"
    );
    Ok(())
}

#[tokio::test]
async fn worker_retries_compress_job_when_ai_fails() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-compress-ai-failure");
    configure_codex_stub("/tmp/remem-missing-codex-for-compress")?;

    let conn = db::open_db()?;
    insert_compressible_observations(&conn, "/tmp/remem", 101)?;
    let job_id = db::enqueue_job(
        &conn,
        "codex-cli",
        db::JobType::Compress,
        "/tmp/remem",
        None,
        "{}",
        200,
    )?;

    run(true, 10).await?;

    let conn = db::open_db()?;
    let (state, attempt_count, next_retry, last_error): (String, i64, Option<i64>, Option<String>) =
        conn.query_row(
            "SELECT state, attempt_count, next_retry_epoch, last_error FROM jobs WHERE id = ?1",
            params![job_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    anyhow::ensure!(state == "pending", "expected pending retry, got {state}");
    anyhow::ensure!(
        attempt_count == 1,
        "expected one attempt, got {attempt_count}"
    );
    anyhow::ensure!(next_retry.is_some(), "expected retry delay");
    anyhow::ensure!(
        last_error
            .as_deref()
            .is_some_and(|err| err.contains("failed to spawn")),
        "expected missing codex path error, got {last_error:?}"
    );
    Ok(())
}

#[tokio::test]
async fn worker_retries_unimplemented_extraction_task() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-extraction-unimplemented");
    let conn = db::open_db()?;
    let outcome = db::record_captured_event(
        &conn,
        &db::CaptureEventInput {
            host: "codex-cli",
            session_id: "sess-extract",
            project: "/tmp/remem",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: r#"{"tool_name":"Bash"}"#,
            task_kind: Some(db::ExtractionTaskKind::RuleCandidate),
        },
    )?;
    let task_id = outcome
        .extraction_task_id
        .expect("capture should coalesce extraction task");

    run(true, 10).await?;

    let conn = db::open_db()?;
    let (status, attempts, next_retry, last_error): (String, i64, Option<i64>, Option<String>) = conn.query_row(
            "SELECT status, attempts, next_retry_epoch, last_error FROM extraction_tasks WHERE id = ?1",
            params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    anyhow::ensure!(status == "pending", "expected pending task, got {status}");
    anyhow::ensure!(attempts == 1, "expected one attempt, got {attempts}");
    anyhow::ensure!(next_retry.is_some(), "expected retry delay");
    anyhow::ensure!(
        last_error
            .as_deref()
            .is_some_and(|err| err.contains("not implemented")),
        "expected explicit unimplemented error"
    );
    Ok(())
}

#[tokio::test]
async fn worker_once_records_startup_heartbeat_without_work() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-once-startup-heartbeat");

    run(true, 10).await?;

    let conn = db::open_db()?;
    let Some(heartbeat) = db::latest_worker_heartbeat(&conn)? else {
        anyhow::bail!("worker --once should record startup heartbeat");
    };
    let expected_prefix = format!("worker-v{}-once-", env!("CARGO_PKG_VERSION"));
    anyhow::ensure!(
        heartbeat.owner.starts_with(&expected_prefix),
        "unexpected heartbeat owner {}",
        heartbeat.owner
    );
    anyhow::ensure!(
        heartbeat.started_at_epoch <= heartbeat.updated_at_epoch,
        "heartbeat should be valid immediately after singleton acquisition"
    );
    Ok(())
}

#[tokio::test]
async fn worker_once_backfills_pending_memory_embeddings() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-once-embedding-backfill");
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value("embeddings.provider", "feature-hash")?;
    let conn = db::open_db()?;
    conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES (1, '/tmp/remem', 'Credential store', 'SQLCipher encrypts secrets at rest.', 'architecture', 1, 1, 'active')",
            [],
        )?;
    drop(conn);

    run(true, 10).await?;

    let conn = db::open_db()?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*)
             FROM memory_embeddings
             WHERE memory_id = 1
               AND model = ?1
               AND dimensions = ?2",
        params![
            crate::retrieval::embedding::FEATURE_HASH_EMBEDDING_MODEL,
            crate::retrieval::embedding::FEATURE_HASH_EMBEDDING_DIMENSIONS as i64
        ],
        |row| row.get(0),
    )?;
    anyhow::ensure!(count == 1, "worker should backfill one embedding row");
    Ok(())
}

#[tokio::test]
async fn worker_once_refreshes_heartbeat_in_drain_loop() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-once-loop-heartbeat");
    let conn = db::open_db()?;
    conn.execute_batch(
        "CREATE TABLE heartbeat_updates (owner TEXT NOT NULL);
             CREATE TRIGGER record_worker_heartbeat_update
             AFTER UPDATE ON worker_heartbeats
             BEGIN
                 INSERT INTO heartbeat_updates (owner) VALUES (new.owner);
             END;",
    )?;
    drop(conn);

    run(true, 10).await?;

    let conn = db::open_db()?;
    let update_count: i64 = conn.query_row(
        "SELECT COUNT(*)
             FROM heartbeat_updates
             WHERE owner LIKE 'worker-once-%'
                OR owner LIKE 'worker-v%-once-%'",
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        update_count >= 1,
        "worker --once should refresh its heartbeat inside the drain loop"
    );
    Ok(())
}

#[tokio::test]
async fn worker_once_exits_without_work_when_singleton_lock_is_held() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-once-lock-held");
    let Some(_guard) = lock::acquire_worker_singleton()? else {
        anyhow::bail!("test worker lock should acquire");
    };
    let conn = db::open_db()?;
    let job_id = db::enqueue_job(
        &conn,
        "codex-cli",
        db::JobType::Observation,
        "/tmp/remem",
        Some("sess-lock-held"),
        r#"{"host":"codex-cli","session_id":"sess-lock-held","project":"/tmp/remem"}"#,
        50,
    )?;

    run(true, 10).await?;

    let conn = db::open_db()?;
    let state: String = conn.query_row(
        "SELECT state FROM jobs WHERE id = ?1",
        params![job_id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        state == "pending",
        "locked-out worker should not process job"
    );
    anyhow::ensure!(
        db::latest_worker_heartbeat(&conn)?.is_none(),
        "locked-out worker should exit before recording heartbeat"
    );
    Ok(())
}

#[tokio::test]
async fn old_version_daemon_lock_allows_current_once_heartbeat() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-daemon-once-mutual-exclusion");
    let Some(_daemon_lock) = lock::acquire_worker_singleton()? else {
        anyhow::bail!("test daemon lock should acquire");
    };
    let conn = db::open_db()?;
    let now = chrono::Utc::now().timestamp();
    let daemon_owner = "worker-daemon-test";
    db::upsert_worker_heartbeat(&conn, daemon_owner, i64::from(std::process::id()), now, now)?;

    run(true, 10).await?;

    let expected_prefix = format!("worker-v{}-once-", env!("CARGO_PKG_VERSION"));
    let current_once_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM worker_heartbeats WHERE owner LIKE ?1",
        params![format!("{expected_prefix}%")],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
            current_once_count == 1,
            "worker --once should record one current-version heartbeat after bypass, got {current_once_count}"
        );
    Ok(())
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn worker_processes_session_rollup_task_on_codex_stub() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-session-rollup");
    let stub_codex = std::env::temp_dir().join(format!(
        "remem-test-codex-rollup-{}-{}.sh",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    install_stub_codex(&stub_codex);
    let stub_codex_str = stub_codex
        .as_os_str()
        .to_str()
        .expect("stub codex path should be valid utf-8");
    configure_codex_stub(stub_codex_str)?;

    let conn = db::open_db()?;
    let outcome = db::record_captured_event(
        &conn,
        &db::CaptureEventInput {
            host: "codex-cli",
            session_id: "sess-rollup-worker",
            project: "/tmp/remem",
            cwd: None,
            event_type: "session_stop",
            role: None,
            tool_name: None,
            content: r#"{"session_id":"sess-rollup-worker","result":"done"}"#,
            task_kind: Some(db::ExtractionTaskKind::SessionRollup),
        },
    )?;
    let task_id = outcome
        .extraction_task_id
        .expect("capture should coalesce extraction task");

    let test_result = async {
        run(true, 10).await?;

        let conn = db::open_db()?;
        let (status, cursor, high_watermark): (String, Option<i64>, Option<i64>) = conn.query_row(
            "SELECT status, cursor_event_id, high_watermark_event_id
                     FROM extraction_tasks WHERE id = ?1",
            params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let summary_text: String = conn.query_row(
            "SELECT summary_text FROM session_summaries
                 WHERE session_row_id IS NOT NULL",
            [],
            |row| row.get(0),
        )?;

        anyhow::ensure!(status == "done", "expected done task, got {status}");
        anyhow::ensure!(cursor == high_watermark, "expected cursor to advance");
        anyhow::ensure!(
            summary_text.contains("Codex worker flush"),
            "expected stub summary text"
        );
        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = std::fs::remove_file(&stub_codex);
    test_result
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn worker_processes_observation_extract_task_on_codex_stub() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-observation-extract");
    let stub_codex = std::env::temp_dir().join(format!(
        "remem-test-codex-observation-{}-{}.sh",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    install_stub_codex(&stub_codex);
    let stub_codex_str = stub_codex
        .as_os_str()
        .to_str()
        .expect("stub codex path should be valid utf-8");
    configure_codex_stub(stub_codex_str)?;

    let conn = db::open_db()?;
    let outcome = db::record_captured_event(
        &conn,
        &db::CaptureEventInput {
            host: "codex-cli",
            session_id: "sess-observe-worker",
            project: "/tmp/remem",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: r#"{"tool_name":"Bash","output":"important"}"#,
            task_kind: Some(db::ExtractionTaskKind::ObservationExtract),
        },
    )?;
    let task_id = outcome
        .extraction_task_id
        .expect("capture should coalesce extraction task");

    let test_result = async {
        run(true, 10).await?;

        let conn = db::open_db()?;
        let (status, cursor, high_watermark): (String, Option<i64>, Option<i64>) = conn.query_row(
            "SELECT status, cursor_event_id, high_watermark_event_id
                     FROM extraction_tasks WHERE id = ?1",
            params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let (text, evidence): (String, String) = conn.query_row(
            "SELECT text, evidence_event_ids FROM observations
                 WHERE session_row_id IS NOT NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let pending_memory_candidates: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_candidates WHERE review_status = 'pending_review'",
            [],
            |row| row.get(0),
        )?;
        let (graph_status, graph_attempts, graph_error): (String, i64, Option<String>) = conn
            .query_row(
                "SELECT status, attempts, last_error
                     FROM extraction_tasks
                     WHERE task_kind = ?1",
                params![db::ExtractionTaskKind::GraphCandidate.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;

        anyhow::ensure!(status == "done", "expected done task, got {status}");
        anyhow::ensure!(cursor == high_watermark, "expected cursor to advance");
        anyhow::ensure!(
            text.contains("Queued Codex observation persisted"),
            "expected stub observation text"
        );
        anyhow::ensure!(evidence.contains('1'), "expected captured event evidence");
        anyhow::ensure!(
            pending_memory_candidates == 1,
            "expected pending memory candidate review"
        );
        anyhow::ensure!(
            graph_status == "pending",
            "expected graph task pending after memory review gate, got {graph_status}"
        );
        anyhow::ensure!(
            graph_attempts == 0,
            "expected graph task to wait without consuming attempts, got {graph_attempts}"
        );
        anyhow::ensure!(
            graph_error
                .as_deref()
                .is_some_and(|err| err.contains("pending review")),
            "expected graph task defer reason to mention pending review"
        );
        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = std::fs::remove_file(&stub_codex);
    test_result
}

#[tokio::test]
async fn worker_heartbeat_updates_in_loop() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-heartbeat-loop");

    let timed = tokio::time::timeout(std::time::Duration::from_millis(40), run(false, 10)).await;
    anyhow::ensure!(timed.is_err(), "daemon worker should keep running");
    let conn = db::open_db()?;
    let heartbeat = db::latest_worker_heartbeat(&conn)?;
    let heartbeat = heartbeat.expect("daemon worker should emit heartbeat");
    anyhow::ensure!(
        heartbeat.owner.starts_with("worker-"),
        "unexpected heartbeat owner {}",
        heartbeat.owner
    );
    anyhow::ensure!(
        heartbeat.updated_at_epoch >= heartbeat.started_at_epoch,
        "heartbeat should advance updated_at_epoch"
    );
    Ok(())
}

fn configure_codex_stub(stub_codex: &str) -> anyhow::Result<()> {
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value("memory_ai.profiles.codex.path", stub_codex)?;
    Ok(())
}

fn insert_compressible_observations(
    conn: &rusqlite::Connection,
    project: &str,
    count: usize,
) -> anyhow::Result<()> {
    for idx in 0..count {
        db::insert_observation(
            conn,
            &format!("compress-source-{idx}"),
            project,
            "discovery",
            Some(&format!("Source {idx}")),
            None,
            Some(&format!("Source observation {idx}")),
            None,
            None,
            None,
            None,
            None,
            0,
        )?;
    }
    Ok(())
}
