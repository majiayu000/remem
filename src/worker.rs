use anyhow::Result;
use tokio::time::{sleep, Duration};

use crate::{db, summarize};

mod lock;

// The lease is the maximum time another worker will wait before requeuing a
// job whose owner died, so `JOB_LEASE_SECS` must always exceed
// `JOB_TIMEOUT_SECS`. Otherwise a job that legitimately runs near the
// timeout could be claimed by a second worker before its current owner has
// given up, causing duplicate processing on hard kills. The grace window
// (60s) gives the active worker time to fail the timeout check and release.
const JOB_TIMEOUT_SECS: u64 = 420;
const JOB_LEASE_SECS: i64 = (JOB_TIMEOUT_SECS as i64) + 60;
const _: () = assert!(JOB_LEASE_SECS > JOB_TIMEOUT_SECS as i64);
const EXTRACTION_TASK_TIMEOUT_SECS: u64 = JOB_TIMEOUT_SECS;
const EMBEDDING_BACKFILL_IDLE_BATCH_SIZE: i64 = 128;

fn retry_backoff_secs(attempt: i64) -> i64 {
    match attempt {
        0 => 5,
        1 => 15,
        2 => 45,
        3 => 120,
        4 => 300,
        _ => 900,
    }
}

fn record_worker_heartbeat(
    conn: &rusqlite::Connection,
    lease_owner: &str,
    started_at_epoch: i64,
) -> Result<()> {
    db::upsert_worker_heartbeat(
        conn,
        lease_owner,
        i64::from(std::process::id()),
        started_at_epoch,
        chrono::Utc::now().timestamp(),
    )
}

async fn process_job(job: &db::Job) -> Result<()> {
    match job.job_type {
        db::JobType::Observation => {
            crate::log::warn(
                "worker",
                &format!(
                    "skipping legacy observation job id={}; captures are processed via extraction_tasks",
                    job.id
                ),
            );
            Ok(())
        }
        db::JobType::Summary => {
            summarize::process_summary_job_input(&job.host, None, &job.payload_json).await?;
            Ok(())
        }
        db::JobType::Compress => {
            let profile = job_profile(&job.payload_json);
            summarize::process_compress_job(&job.host, &job.project, profile.as_deref()).await?;
            Ok(())
        }
        db::JobType::Dream => {
            let profile = job_profile(&job.payload_json);
            if let Some(profile) = profile.as_deref() {
                crate::dream::process_dream_job_with_profile(&job.project, Some(profile)).await?;
            } else {
                crate::dream::process_dream_job_with_host(&job.project, &job.host).await?;
            }
            Ok(())
        }
    }
}

fn job_profile(payload_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(payload_json)
        .ok()
        .and_then(|value| {
            value
                .get("remem_ai_profile")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|profile| !profile.is_empty())
                .map(str::to_string)
        })
}

fn run_idle_embedding_backfill(conn: &rusqlite::Connection) -> Result<bool> {
    match crate::retrieval::vector::reindex_memory_embeddings_with_report(
        conn,
        EMBEDDING_BACKFILL_IDLE_BATCH_SIZE,
    ) {
        Ok(report) if report.processed > 0 => {
            crate::log::info(
                "worker",
                &format!(
                    "backfilled {} memory embedding(s) for model={} dimensions={}",
                    report.processed, report.model, report.dimensions
                ),
            );
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(error)
            if crate::retrieval::embedding::is_local_embedding_model_unavailable_error(&error) =>
        {
            crate::log::error(
                "worker",
                &format!("memory embedding backfill deferred: {error}"),
            );
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

pub async fn run(once: bool, idle_sleep_ms: u64) -> Result<()> {
    let started_at_epoch = chrono::Utc::now().timestamp();
    let mode = if once { "once" } else { "daemon" };
    let lease_owner = format!(
        "worker-{}-{}-{}",
        mode,
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    );
    let Some(_singleton) = lock::acquire_worker_singleton()? else {
        crate::log::info("worker", "worker already running, exiting");
        return Ok(());
    };
    crate::log::info(
        "worker",
        &format!("start owner={} mode={}", lease_owner, mode),
    );
    {
        let conn = db::open_db()?;
        record_worker_heartbeat(&conn, &lease_owner, started_at_epoch)?;
    }

    loop {
        let mut conn = db::open_db()?;
        record_worker_heartbeat(&conn, &lease_owner, started_at_epoch)?;
        let recovered = db::requeue_stuck_jobs(&conn)?;
        if recovered > 0 {
            crate::log::warn("worker", &format!("requeued {} stuck job(s)", recovered));
        }
        let recovered_extraction = db::release_expired_extraction_task_leases(&conn)?;
        if recovered_extraction > 0 {
            crate::log::warn(
                "worker",
                &format!(
                    "released {} expired extraction task lease(s)",
                    recovered_extraction
                ),
            );
        }
        db::maintain_failure_lifecycle(&conn)?;
        if let Some(job) = db::claim_next_job(&mut conn, &lease_owner, JOB_LEASE_SECS)? {
            crate::log::info(
                "worker",
                &format!(
                    "claimed id={} type={} project={} attempt={}/{}",
                    job.id,
                    job.job_type.as_str(),
                    job.project,
                    job.attempt_count + 1,
                    job.max_attempts
                ),
            );

            let timed =
                tokio::time::timeout(Duration::from_secs(JOB_TIMEOUT_SECS), process_job(&job))
                    .await;
            let conn = db::open_db()?;
            match timed {
                Ok(Ok(())) => {
                    db::mark_job_done(&conn, job.id, &lease_owner)?;
                    crate::log::info("worker", &format!("done id={}", job.id));
                }
                Ok(Err(e)) => {
                    let msg = e.to_string();
                    let backoff = retry_backoff_secs(job.attempt_count);
                    db::mark_job_failed_or_retry(&conn, job.id, &lease_owner, &msg, backoff)?;
                    crate::log::warn(
                        "worker",
                        &format!(
                            "job id={} failed: {} (retry in {}s)",
                            job.id,
                            crate::db::truncate_str(&msg, 300),
                            backoff
                        ),
                    );
                }
                Err(_) => {
                    let msg = format!("job timed out after {}s", JOB_TIMEOUT_SECS);
                    let backoff = retry_backoff_secs(job.attempt_count);
                    db::mark_job_failed_or_retry(&conn, job.id, &lease_owner, &msg, backoff)?;
                    crate::log::warn(
                        "worker",
                        &format!("job id={} timeout (retry in {}s)", job.id, backoff),
                    );
                }
            }
            continue;
        }

        if crate::extraction_worker::run_next(
            &lease_owner,
            JOB_LEASE_SECS,
            EXTRACTION_TASK_TIMEOUT_SECS,
        )
        .await?
        {
            continue;
        }

        if run_idle_embedding_backfill(&conn)? {
            continue;
        }

        if once {
            break;
        }
        sleep(Duration::from_millis(idle_sleep_ms.max(100))).await;
        continue;
    }

    if !once {
        let conn = db::open_db()?;
        record_worker_heartbeat(&conn, &lease_owner, started_at_epoch)?;
    }
    crate::log::info("worker", "stopped");
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
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
        let (state, attempt_count, next_retry, last_error): (
            String,
            i64,
            Option<i64>,
            Option<String>,
        ) = conn.query_row(
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
        anyhow::ensure!(
            heartbeat.owner.starts_with("worker-once-"),
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
            "SELECT COUNT(*) FROM heartbeat_updates WHERE owner LIKE 'worker-once-%'",
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
    async fn daemon_and_once_are_mutually_exclusive() -> anyhow::Result<()> {
        let _data_dir = ScopedTestDataDir::new("worker-daemon-once-mutual-exclusion");
        let Some(_daemon_lock) = lock::acquire_worker_singleton()? else {
            anyhow::bail!("test daemon lock should acquire");
        };
        let conn = db::open_db()?;
        let now = chrono::Utc::now().timestamp();
        let daemon_owner = "worker-daemon-test";
        db::upsert_worker_heartbeat(&conn, daemon_owner, i64::from(std::process::id()), now, now)?;

        run(true, 10).await?;

        let conn = db::open_db()?;
        let Some(heartbeat) = db::latest_worker_heartbeat(&conn)? else {
            anyhow::bail!("daemon heartbeat should remain present");
        };
        anyhow::ensure!(
            heartbeat.owner == daemon_owner,
            "worker --once should not replace daemon heartbeat while daemon holds singleton"
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
            let (status, cursor, high_watermark): (String, Option<i64>, Option<i64>) = conn
                .query_row(
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
            let (status, cursor, high_watermark): (String, Option<i64>, Option<i64>) = conn
                .query_row(
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

        let timed =
            tokio::time::timeout(std::time::Duration::from_millis(40), run(false, 10)).await;
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
}
