use anyhow::Result;
use serde::Deserialize;
use tokio::time::{sleep, Duration};

use crate::{db, observe_flush, summarize};

const JOB_LEASE_SECS: i64 = 600;
const JOB_TIMEOUT_SECS: u64 = 420;

#[derive(Debug, Deserialize)]
struct ObservationPayload {
    session_id: String,
    project: String,
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

async fn process_job(job: &db::Job) -> Result<()> {
    match job.job_type {
        db::JobType::Observation => {
            let payload: ObservationPayload = serde_json::from_str(&job.payload_json)?;
            let _ = observe_flush::flush_pending(&payload.session_id, &payload.project).await?;
            Ok(())
        }
        db::JobType::Summary => summarize::process_summary_job_input(&job.payload_json).await,
        db::JobType::Compress => summarize::process_compress_job(&job.project).await,
        db::JobType::Dream => crate::dream::process_dream_job(&job.project).await,
    }
}

pub async fn run(once: bool, idle_sleep_ms: u64) -> Result<()> {
    let lease_owner = format!(
        "worker-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    );
    crate::log::info("worker", &format!("start owner={}", lease_owner));

    loop {
        let mut conn = db::open_db()?;
        let recovered = db::requeue_stuck_jobs(&conn)?;
        if recovered > 0 {
            crate::log::warn("worker", &format!("requeued {} stuck job(s)", recovered));
        }

        let Some(job) = db::claim_next_job(&mut conn, &lease_owner, JOB_LEASE_SECS)? else {
            if once {
                break;
            }
            sleep(Duration::from_millis(idle_sleep_ms.max(100))).await;
            continue;
        };

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
            tokio::time::timeout(Duration::from_secs(JOB_TIMEOUT_SECS), process_job(&job)).await;
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
            ("REMEM_SUMMARY_EXECUTOR", Some("codex-cli")),
            ("REMEM_FLUSH_EXECUTOR", None),
            ("REMEM_CODEX_PATH", Some(stub_codex_str)),
            ("REMEM_CLAUDE_PATH", Some("/definitely/missing/claude")),
            ("ANTHROPIC_API_KEY", None),
            ("ANTHROPIC_AUTH_TOKEN", None),
        ]);

        let conn = db::open_db()?;
        db::enqueue_pending(
            &conn,
            "sess-codex",
            "proj-codex",
            "Bash",
            Some("echo codex"),
            Some("codex output"),
            None,
        )?;
        let job_id = db::enqueue_job(
            &conn,
            db::JobType::Observation,
            "proj-codex",
            Some("sess-codex"),
            r#"{"session_id":"sess-codex","project":"proj-codex"}"#,
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
}
