use anyhow::Result;
use serde::Deserialize;
use std::ffi::OsString;
use tokio::time::{sleep, Duration};

use crate::{db, observe, summarize};

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
const STALE_PENDING_AGE_SECS: i64 = 60;
const STALE_PENDING_SCAN_LIMIT: i64 = 8;

#[derive(Debug, Deserialize)]
struct ObservationPayload {
    host: Option<String>,
    session_id: String,
    project: String,
}

#[derive(Debug, Clone)]
enum JobOutcome {
    Done,
    ObservationNeedsFollowUp {
        host: String,
        session_id: String,
        project: String,
        payload_json: String,
    },
}

struct ScopedExecutorEnv {
    previous: Vec<(&'static str, Option<OsString>)>,
}

impl ScopedExecutorEnv {
    fn for_job(job: &db::Job) -> Self {
        let Some(executor) = executor_for_host(&job.host) else {
            return Self {
                previous: Vec::new(),
            };
        };
        let keys: &[&'static str] = match job.job_type {
            db::JobType::Observation => &["REMEM_FLUSH_EXECUTOR"],
            db::JobType::Summary => &["REMEM_SUMMARY_EXECUTOR"],
            db::JobType::Compress | db::JobType::Dream => &[],
        };
        let previous = keys
            .iter()
            .map(|key| {
                let old = std::env::var_os(key);
                unsafe { std::env::set_var(key, executor) };
                (*key, old)
            })
            .collect();
        Self { previous }
    }
}

impl Drop for ScopedExecutorEnv {
    fn drop(&mut self) {
        for (key, old) in self.previous.iter().rev() {
            match old {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}

fn executor_for_host(host: &str) -> Option<&'static str> {
    match host {
        "codex-cli" => Some("codex-cli"),
        "claude-code" => Some("claude-cli"),
        _ => None,
    }
}

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

fn enqueue_stale_observation_jobs(conn: &rusqlite::Connection) -> Result<usize> {
    let identities =
        db::get_stale_pending_identities(conn, STALE_PENDING_AGE_SECS, STALE_PENDING_SCAN_LIMIT)?;
    let mut queued = 0usize;
    for identity in identities {
        let payload = serde_json::json!({
            "host": identity.host.as_str(),
            "session_id": identity.session_id.as_str(),
            "project": identity.project.as_str(),
        });
        db::enqueue_job(
            conn,
            &identity.host,
            db::JobType::Observation,
            &identity.project,
            Some(&identity.session_id),
            &payload.to_string(),
            observe::flush::OBSERVATION_FOLLOW_UP_PRIORITY,
        )?;
        queued += 1;
    }
    Ok(queued)
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

async fn process_job(job: &db::Job) -> Result<JobOutcome> {
    match job.job_type {
        db::JobType::Observation => {
            let payload: ObservationPayload = serde_json::from_str(&job.payload_json)?;
            let host = payload.host.unwrap_or_else(|| job.host.clone());
            let outcome =
                observe::flush::flush_pending(&host, &payload.session_id, &payload.project).await?;
            match outcome {
                observe::flush::ObservationDrainOutcome::Drained => Ok(JobOutcome::Done),
                observe::flush::ObservationDrainOutcome::NeedsFollowUp => {
                    Ok(JobOutcome::ObservationNeedsFollowUp {
                        host,
                        session_id: payload.session_id,
                        project: payload.project,
                        payload_json: job.payload_json.clone(),
                    })
                }
            }
        }
        db::JobType::Summary => {
            summarize::process_summary_job_input(&job.payload_json).await?;
            Ok(JobOutcome::Done)
        }
        db::JobType::Compress => {
            summarize::process_compress_job(&job.project).await?;
            Ok(JobOutcome::Done)
        }
        db::JobType::Dream => {
            crate::dream::process_dream_job(&job.project).await?;
            Ok(JobOutcome::Done)
        }
    }
}

pub async fn run(once: bool, idle_sleep_ms: u64) -> Result<()> {
    let started_at_epoch = chrono::Utc::now().timestamp();
    let lease_owner = format!(
        "worker-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    );
    crate::log::info("worker", &format!("start owner={}", lease_owner));

    loop {
        let mut conn = db::open_db()?;
        if !once {
            record_worker_heartbeat(&conn, &lease_owner, started_at_epoch)?;
        }
        let recovered = db::requeue_stuck_jobs(&conn)?;
        if recovered > 0 {
            crate::log::warn("worker", &format!("requeued {} stuck job(s)", recovered));
        }
        let recovered_pending = db::release_expired_pending_claims(&conn)?;
        if recovered_pending > 0 {
            crate::log::warn(
                "worker",
                &format!("released {} expired pending claim(s)", recovered_pending),
            );
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
        let queued_stale = enqueue_stale_observation_jobs(&conn)?;
        if queued_stale > 0 {
            crate::log::info(
                "worker",
                &format!("queued {} stale observation job(s)", queued_stale),
            );
        }

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

            let scoped_executor = ScopedExecutorEnv::for_job(&job);
            let timed =
                tokio::time::timeout(Duration::from_secs(JOB_TIMEOUT_SECS), process_job(&job))
                    .await;
            drop(scoped_executor);
            let conn = db::open_db()?;
            match timed {
                Ok(Ok(outcome)) => {
                    db::mark_job_done(&conn, job.id, &lease_owner)?;
                    crate::log::info("worker", &format!("done id={}", job.id));
                    if let JobOutcome::ObservationNeedsFollowUp {
                        host,
                        session_id,
                        project,
                        payload_json,
                    } = outcome
                    {
                        let follow_up_id = db::enqueue_job(
                            &conn,
                            &host,
                            db::JobType::Observation,
                            &project,
                            Some(&session_id),
                            &payload_json,
                            observe::flush::OBSERVATION_FOLLOW_UP_PRIORITY,
                        )?;
                        crate::log::info(
                            "worker",
                            &format!("queued observation follow-up id={}", follow_up_id),
                        );
                    }
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
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Mutex, MutexGuard};

    use rusqlite::params;

    use crate::db::{self, test_support::ScopedTestDataDir};

    use super::run;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: Option<&str>) -> Self {
            let previous = std::env::var_os(key);
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            match self.previous.as_ref() {
                Some(value) => unsafe { std::env::set_var(self.key, value) },
                None => unsafe { std::env::remove_var(self.key) },
            }
        }
    }

    struct ScopedEnv {
        _guard: MutexGuard<'static, ()>,
        _vars: Vec<ScopedEnvVar>,
    }

    impl ScopedEnv {
        fn set(vars: &[(&'static str, Option<&str>)]) -> Self {
            let guard = ENV_LOCK.lock().expect("env lock should acquire");
            let vars = vars
                .iter()
                .map(|(key, value)| ScopedEnvVar::set(key, *value))
                .collect();
            Self {
                _guard: guard,
                _vars: vars,
            }
        }
    }

    fn install_stub_codex(path: &std::path::Path) {
        let script = r#"#!/bin/sh
prev=""
output_path=""
for arg in "$@"; do
  if [ "$prev" = "--output-last-message" ]; then
    output_path="$arg"
    break
  fi
  prev="$arg"
done
cat >/dev/null
if [ -z "$output_path" ]; then
  echo "missing output path" >&2
  exit 1
fi
cat <<'EOF' > "$output_path"
<observation>
  <type>decision</type>
  <title>Codex worker flush</title>
  <narrative>Queued Codex observation persisted.</narrative>
</observation>
EOF
"#;
        std::fs::write(path, script).expect("stub codex script should be written");
        let mut perms = std::fs::metadata(path)
            .expect("stub codex metadata should load")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("stub codex permissions should be set");
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn worker_processes_observation_job_on_codex_only_env() -> anyhow::Result<()> {
        let _data_dir = ScopedTestDataDir::new("worker-codex-flush");
        let stub_codex = std::env::temp_dir().join(format!(
            "remem-test-codex-{}-{}.sh",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        install_stub_codex(&stub_codex);
        let stub_codex_str = stub_codex
            .as_os_str()
            .to_str()
            .expect("stub codex path should be valid utf-8");
        let _env = ScopedEnv::set(&[
            ("REMEM_EXECUTOR", None),
            ("REMEM_SUMMARY_EXECUTOR", None),
            ("REMEM_FLUSH_EXECUTOR", None),
            ("REMEM_CODEX_PATH", Some(stub_codex_str)),
            ("REMEM_CLAUDE_PATH", Some("/definitely/missing/claude")),
            ("ANTHROPIC_API_KEY", None),
            ("ANTHROPIC_AUTH_TOKEN", None),
        ]);

        let conn = db::open_db()?;
        db::enqueue_pending(
            &conn,
            "codex-cli",
            "sess-codex",
            "proj-codex",
            "Bash",
            Some("echo codex"),
            Some("codex output"),
            None,
        )?;
        let job_id = db::enqueue_job(
            &conn,
            "codex-cli",
            db::JobType::Observation,
            "proj-codex",
            Some("sess-codex"),
            r#"{"host":"codex-cli","session_id":"sess-codex","project":"proj-codex"}"#,
            50,
        )?;

        let test_result = async {
            run(true, 10).await?;

            let conn = db::open_db()?;
            let observation_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM observations WHERE project = ?1",
                params!["proj-codex"],
                |row| row.get(0),
            )?;
            let pending_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM pending_observations WHERE session_id = ?1",
                params!["sess-codex"],
                |row| row.get(0),
            )?;
            let job_state: String = conn.query_row(
                "SELECT state FROM jobs WHERE id = ?1",
                params![job_id],
                |row| row.get(0),
            )?;
            let flush_executor: String = conn.query_row(
                "SELECT executor FROM ai_usage_events WHERE operation = 'flush' ORDER BY created_at_epoch DESC LIMIT 1",
                [],
                |row| row.get(0),
            )?;

            anyhow::ensure!(observation_count > 0, "expected persisted observations");
            anyhow::ensure!(pending_count == 0, "expected pending queue to be drained");
            anyhow::ensure!(job_state == "done", "expected observation job done, got {job_state}");
            anyhow::ensure!(
                flush_executor == "codex-cli",
                "expected codex flush executor, got {flush_executor}"
            );

            Ok::<(), anyhow::Error>(())
        }
        .await;

        let _ = std::fs::remove_file(&stub_codex);
        test_result
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn worker_drains_multiple_observation_batches() -> anyhow::Result<()> {
        let _data_dir = ScopedTestDataDir::new("worker-drain-multiple");
        let stub_codex = std::env::temp_dir().join(format!(
            "remem-test-codex-drain-{}-{}.sh",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        install_stub_codex(&stub_codex);
        let stub_codex_str = stub_codex
            .as_os_str()
            .to_str()
            .expect("stub codex path should be valid utf-8");
        let _env = ScopedEnv::set(&[
            ("REMEM_EXECUTOR", None),
            ("REMEM_SUMMARY_EXECUTOR", None),
            ("REMEM_FLUSH_EXECUTOR", None),
            ("REMEM_CODEX_PATH", Some(stub_codex_str)),
            ("REMEM_CLAUDE_PATH", Some("/definitely/missing/claude")),
            ("ANTHROPIC_API_KEY", None),
            ("ANTHROPIC_AUTH_TOKEN", None),
        ]);

        let conn = db::open_db()?;
        for idx in 0..20 {
            db::enqueue_pending(
                &conn,
                "codex-cli",
                "sess-drain",
                "proj-drain",
                "Bash",
                Some(&format!("echo codex {idx}")),
                Some("codex output"),
                None,
            )?;
        }
        let job_id = db::enqueue_job(
            &conn,
            "codex-cli",
            db::JobType::Observation,
            "proj-drain",
            Some("sess-drain"),
            r#"{"host":"codex-cli","session_id":"sess-drain","project":"proj-drain"}"#,
            50,
        )?;

        let test_result = async {
            run(true, 10).await?;

            let conn = db::open_db()?;
            let pending_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM pending_observations WHERE session_id = ?1",
                params!["sess-drain"],
                |row| row.get(0),
            )?;
            let observation_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM observations WHERE project = ?1",
                params!["proj-drain"],
                |row| row.get(0),
            )?;
            let flush_calls: i64 = conn.query_row(
                "SELECT COUNT(*) FROM ai_usage_events WHERE operation = 'flush'",
                [],
                |row| row.get(0),
            )?;
            let job_state: String = conn.query_row(
                "SELECT state FROM jobs WHERE id = ?1",
                params![job_id],
                |row| row.get(0),
            )?;

            anyhow::ensure!(pending_count == 0, "expected all pending rows to drain");
            anyhow::ensure!(observation_count > 0, "expected persisted observations");
            anyhow::ensure!(flush_calls >= 2, "expected multiple flush batches");
            anyhow::ensure!(job_state == "done", "expected job done, got {job_state}");

            Ok::<(), anyhow::Error>(())
        }
        .await;

        let _ = std::fs::remove_file(&stub_codex);
        test_result
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn worker_schedules_stale_pending_observation_job() -> anyhow::Result<()> {
        let _data_dir = ScopedTestDataDir::new("worker-stale-scheduler");
        let stub_codex = std::env::temp_dir().join(format!(
            "remem-test-codex-stale-{}-{}.sh",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        install_stub_codex(&stub_codex);
        let stub_codex_str = stub_codex
            .as_os_str()
            .to_str()
            .expect("stub codex path should be valid utf-8");
        let _env = ScopedEnv::set(&[
            ("REMEM_EXECUTOR", None),
            ("REMEM_SUMMARY_EXECUTOR", None),
            ("REMEM_FLUSH_EXECUTOR", None),
            ("REMEM_CODEX_PATH", Some(stub_codex_str)),
            ("REMEM_CLAUDE_PATH", Some("/definitely/missing/claude")),
            ("ANTHROPIC_API_KEY", None),
            ("ANTHROPIC_AUTH_TOKEN", None),
        ]);

        let conn = db::open_db()?;
        let pending_id = db::enqueue_pending(
            &conn,
            "codex-cli",
            "sess-stale",
            "proj-stale",
            "Bash",
            Some("echo stale"),
            Some("codex output"),
            None,
        )?;
        conn.execute(
            "UPDATE pending_observations
             SET created_at_epoch = ?2, updated_at_epoch = ?2
             WHERE id = ?1",
            params![pending_id, chrono::Utc::now().timestamp() - 120],
        )?;

        let test_result = async {
            run(true, 10).await?;

            let conn = db::open_db()?;
            let pending_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM pending_observations WHERE session_id = ?1",
                params!["sess-stale"],
                |row| row.get(0),
            )?;
            let done_jobs: i64 = conn.query_row(
                "SELECT COUNT(*) FROM jobs
                 WHERE job_type = 'observation' AND state = 'done' AND session_id = ?1",
                params!["sess-stale"],
                |row| row.get(0),
            )?;

            anyhow::ensure!(pending_count == 0, "expected stale pending row to drain");
            anyhow::ensure!(done_jobs == 1, "expected one scheduled observation job");

            Ok::<(), anyhow::Error>(())
        }
        .await;

        let _ = std::fs::remove_file(&stub_codex);
        test_result
    }

    #[tokio::test]
    async fn worker_defers_unimplemented_extraction_task() -> anyhow::Result<()> {
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
                task_kind: Some(db::ExtractionTaskKind::ObservationExtract),
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
        anyhow::ensure!(attempts == 0, "expected zero attempts, got {attempts}");
        anyhow::ensure!(next_retry.is_some(), "expected retry delay");
        anyhow::ensure!(
            last_error
                .as_deref()
                .is_some_and(|err| err.contains("not implemented")),
            "expected explicit unimplemented error"
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
        let _env = ScopedEnv::set(&[
            ("REMEM_EXECUTOR", Some("codex-cli")),
            ("REMEM_SUMMARY_EXECUTOR", None),
            ("REMEM_FLUSH_EXECUTOR", None),
            ("REMEM_CODEX_PATH", Some(stub_codex_str)),
            ("REMEM_CLAUDE_PATH", Some("/definitely/missing/claude")),
            ("ANTHROPIC_API_KEY", None),
            ("ANTHROPIC_AUTH_TOKEN", None),
        ]);

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
}
