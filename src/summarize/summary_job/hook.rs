use anyhow::Result;

use crate::db;

use super::super::constants::SUMMARIZE_STDIN_TIMEOUT_MS;
use super::super::input::{read_stdin_with_timeout, SummarizeInput};

pub async fn summarize() -> Result<()> {
    let Some(input) = read_stdin_with_timeout(SUMMARIZE_STDIN_TIMEOUT_MS)? else {
        return Ok(());
    };

    let hook: SummarizeInput = match serde_json::from_str(&input) {
        Ok(value) => value,
        Err(err) => {
            crate::log::warn(
                "summarize",
                &format!("invalid hook payload, skipping: {}", err),
            );
            return Ok(());
        }
    };
    let Some(session_id) = &hook.session_id else {
        return Ok(());
    };
    let cwd = effective_cwd(&hook)?;
    let project = db::project_from_cwd(&cwd);
    let host = resolve_hook_host();
    let conn = db::open_db()?;
    let summary_payload = summary_payload_with_cwd(&input, &cwd)?;

    record_summary_capture_event(&conn, &host, session_id, &project, &cwd, &summary_payload);
    enqueue_summary_jobs(&conn, &host, session_id, &project, &summary_payload)?;
    if should_spawn_worker_once(&conn)? {
        spawn_worker_once()?;
    } else {
        crate::log::info(
            "summarize",
            "worker daemon heartbeat healthy; skip worker --once",
        );
    }
    Ok(())
}

fn effective_cwd(hook: &SummarizeInput) -> Result<String> {
    if let Some(cwd) = hook.cwd.as_deref().filter(|cwd| !cwd.trim().is_empty()) {
        return Ok(cwd.to_string());
    }
    Ok(std::env::current_dir()?.display().to_string())
}

fn summary_payload_with_cwd(input: &str, cwd: &str) -> Result<String> {
    let mut payload: serde_json::Value = serde_json::from_str(input)?;
    let Some(obj) = payload.as_object_mut() else {
        return Ok(input.to_string());
    };
    let needs_cwd = obj
        .get("cwd")
        .and_then(|value| value.as_str())
        .is_none_or(|value| value.trim().is_empty());
    if needs_cwd {
        obj.insert(
            "cwd".to_string(),
            serde_json::Value::String(cwd.to_string()),
        );
    }
    Ok(serde_json::to_string(&payload)?)
}

fn record_summary_capture_event(
    conn: &rusqlite::Connection,
    host: &str,
    session_id: &str,
    project: &str,
    cwd: &str,
    content: &str,
) -> bool {
    match db::record_captured_event(
        conn,
        &db::CaptureEventInput {
            host,
            session_id,
            project,
            cwd: Some(cwd),
            event_type: "session_stop",
            role: None,
            tool_name: None,
            content,
            task_kind: Some(db::ExtractionTaskKind::SessionRollup),
        },
    ) {
        Ok(_) => true,
        Err(err) => {
            crate::log::warn(
                "summarize",
                &format!(
                    "capture ledger record failed; continuing summary enqueue: {}",
                    err
                ),
            );
            false
        }
    }
}

fn enqueue_summary_jobs(
    conn: &rusqlite::Connection,
    host: &str,
    session_id: &str,
    project: &str,
    input: &str,
) -> Result<()> {
    let ready_pending = db::count_pending_for_identity(conn, host, project, session_id)?;
    if ready_pending > 0 {
        let obs_payload = serde_json::json!({
            "host": host,
            "session_id": session_id,
            "project": project,
        });
        db::enqueue_job(
            conn,
            host,
            db::JobType::Observation,
            project,
            Some(session_id),
            &obs_payload.to_string(),
            50,
        )?;
    }
    db::enqueue_job(
        conn,
        host,
        db::JobType::Summary,
        project,
        Some(session_id),
        input,
        100,
    )?;
    db::enqueue_job(conn, host, db::JobType::Compress, project, None, "{}", 200)?;
    crate::log::info(
        "summarize",
        &format!(
            "QUEUED summary session={} project={} observation_pending={}",
            session_id, project, ready_pending
        ),
    );
    Ok(())
}

fn resolve_hook_host() -> String {
    if let Ok(host) = std::env::var("REMEM_HOOK_HOST") {
        if !host.trim().is_empty() {
            return host;
        }
    }
    if let Ok(host) = std::env::var("REMEM_CONTEXT_HOST") {
        if !host.trim().is_empty() {
            return host;
        }
    }
    if let Ok(executor) = std::env::var("REMEM_SUMMARY_EXECUTOR") {
        match executor.as_str() {
            "codex-cli" => return "codex-cli".to_string(),
            "claude-cli" => return "claude-code".to_string(),
            _ => {}
        }
    }
    "unknown".to_string()
}

fn should_spawn_worker_once(conn: &rusqlite::Connection) -> Result<bool> {
    Ok(db::healthy_worker_heartbeat(conn, db::WORKER_HEARTBEAT_HEALTH_SECS)?.is_none())
}

fn spawn_worker_once() -> Result<()> {
    let exe = std::env::current_exe()?;
    let worker_dir = stable_worker_dir();
    let stderr_file = crate::log::open_log_append();
    let stderr_cfg = match stderr_file {
        Some(file) => std::process::Stdio::from(file),
        None => std::process::Stdio::null(),
    };
    let mut command = std::process::Command::new(&exe);
    command
        .arg("worker")
        .arg("--once")
        .current_dir(&worker_dir)
        .env("REMEM_STDERR_TO_LOG", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(stderr_cfg);
    configure_worker_executor_env(&mut command);
    let _child = command.spawn()?;
    Ok(())
}

fn stable_worker_dir() -> std::path::PathBuf {
    let data_dir = crate::db::data_dir();
    if let Err(err) = std::fs::create_dir_all(&data_dir) {
        crate::log::warn(
            "summarize",
            &format!(
                "failed to create worker dir {}: {}; falling back to temp dir",
                data_dir.display(),
                err
            ),
        );
        return std::env::temp_dir();
    }
    data_dir
}

fn configure_worker_executor_env(command: &mut std::process::Command) {
    if std::env::var_os("REMEM_SUMMARY_EXECUTOR").is_none() {
        if let Some(executor) = std::env::var_os("REMEM_EXECUTOR") {
            command.env("REMEM_SUMMARY_EXECUTOR", executor);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::sync::Mutex;

    use crate::db::{self, test_support::ScopedTestDataDir};

    use super::{
        configure_worker_executor_env, enqueue_summary_jobs, record_summary_capture_event,
        should_spawn_worker_once, stable_worker_dir, summary_payload_with_cwd,
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env_vars<T>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().expect("env lock should acquire");
        let old_values = vars
            .iter()
            .map(|(key, _)| ((*key).to_string(), std::env::var(key).ok()))
            .collect::<Vec<_>>();

        for (key, value) in vars {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }

        let result = f();

        for (key, value) in old_values {
            match value {
                Some(value) => unsafe { std::env::set_var(&key, value) },
                None => unsafe { std::env::remove_var(&key) },
            }
        }

        result
    }

    fn command_env<'a>(command: &'a std::process::Command, key: &str) -> Option<Option<&'a OsStr>> {
        command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new(key))
            .map(|(_, value)| value)
    }

    #[test]
    fn worker_env_translates_legacy_global_executor_without_removing_it() {
        with_env_vars(
            &[
                ("REMEM_EXECUTOR", Some("codex-cli")),
                ("REMEM_SUMMARY_EXECUTOR", None),
            ],
            || {
                let mut command = std::process::Command::new("remem");
                configure_worker_executor_env(&mut command);

                assert_eq!(
                    command_env(&command, "REMEM_SUMMARY_EXECUTOR"),
                    Some(Some(OsStr::new("codex-cli")))
                );
                assert_eq!(command_env(&command, "REMEM_EXECUTOR"), None);
            },
        );
    }

    #[test]
    fn worker_env_preserves_explicit_summary_override_and_global_executor() {
        with_env_vars(
            &[
                ("REMEM_EXECUTOR", Some("http")),
                ("REMEM_SUMMARY_EXECUTOR", Some("codex-cli")),
            ],
            || {
                let mut command = std::process::Command::new("remem");
                configure_worker_executor_env(&mut command);

                assert_eq!(command_env(&command, "REMEM_SUMMARY_EXECUTOR"), None);
                assert_eq!(command_env(&command, "REMEM_EXECUTOR"), None);
            },
        );
    }

    #[test]
    fn summary_payload_with_cwd_fills_missing_cwd() {
        let payload = summary_payload_with_cwd(r#"{"session_id":"sess-cwd"}"#, "/tmp/project")
            .expect("payload should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&payload).expect("payload should parse");

        assert_eq!(parsed["session_id"].as_str(), Some("sess-cwd"));
        assert_eq!(parsed["cwd"].as_str(), Some("/tmp/project"));
    }

    #[test]
    fn summary_payload_with_cwd_preserves_existing_cwd() {
        let payload =
            summary_payload_with_cwd(r#"{"session_id":"sess-cwd","cwd":"/repo"}"#, "/tmp/project")
                .expect("payload should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&payload).expect("payload should parse");

        assert_eq!(parsed["cwd"].as_str(), Some("/repo"));
    }

    #[test]
    fn capture_ledger_failure_does_not_block_legacy_summary_hook() {
        let _test_dir = ScopedTestDataDir::new("summary-legacy-unknown-host");
        let conn = db::open_db().expect("db should open");

        let captured = record_summary_capture_event(
            &conn,
            "unknown",
            "sess-legacy",
            "/tmp/remem",
            "/tmp/remem",
            r#"{"session_id":"sess-legacy","cwd":"/tmp/remem"}"#,
        );
        enqueue_summary_jobs(
            &conn,
            "unknown",
            "sess-legacy",
            "/tmp/remem",
            r#"{"session_id":"sess-legacy","cwd":"/tmp/remem"}"#,
        )
        .expect("legacy summary jobs should still enqueue");

        assert!(!captured);
        let jobs = job_types(&conn);
        assert_eq!(jobs, vec!["summary".to_string(), "compress".to_string()]);
    }

    #[test]
    fn missing_daemon_uses_stop_fallback_spawn() {
        let _test_dir = ScopedTestDataDir::new("summary-missing-daemon");
        let conn = db::open_db().expect("db should open");

        assert!(
            should_spawn_worker_once(&conn).expect("daemon check should run"),
            "missing heartbeat should keep worker --once fallback"
        );
    }

    #[test]
    fn healthy_daemon_skips_stop_spawn() {
        let _test_dir = ScopedTestDataDir::new("summary-healthy-daemon");
        let conn = db::open_db().expect("db should open");
        let now = chrono::Utc::now().timestamp();
        db::upsert_worker_heartbeat(
            &conn,
            "worker-daemon",
            i64::from(std::process::id()),
            now - 5,
            now - 5,
        )
        .expect("heartbeat should insert");

        assert!(
            !should_spawn_worker_once(&conn).expect("daemon check should run"),
            "healthy heartbeat should skip worker --once fallback"
        );
    }

    #[test]
    fn stale_daemon_uses_stop_fallback_spawn() {
        let _test_dir = ScopedTestDataDir::new("summary-stale-daemon");
        let conn = db::open_db().expect("db should open");
        let now = chrono::Utc::now().timestamp();
        db::upsert_worker_heartbeat(&conn, "worker-daemon", 123, now - 900, now - 900)
            .expect("heartbeat should insert");

        assert!(
            should_spawn_worker_once(&conn).expect("daemon check should run"),
            "stale heartbeat should keep worker --once fallback"
        );
    }

    #[test]
    fn stable_worker_dir_uses_data_dir() {
        let data_dir = ScopedTestDataDir::new("summary-worker-dir");

        let got = stable_worker_dir();

        assert_eq!(got, data_dir.path);
        assert!(got.is_dir());
    }

    #[test]
    fn enqueue_summary_jobs_skips_observation_job_when_no_pending_events() {
        let _test_dir = ScopedTestDataDir::new("summary-no-pending-observation");
        let conn = db::open_db().expect("db should open");

        enqueue_summary_jobs(
            &conn,
            "codex-cli",
            "sess-no-pending",
            "/tmp/remem",
            r#"{"session_id":"sess-no-pending"}"#,
        )
        .expect("summary jobs should enqueue");

        let jobs = job_types(&conn);
        assert_eq!(jobs, vec!["summary".to_string(), "compress".to_string()]);
    }

    #[test]
    fn enqueue_summary_jobs_keeps_observation_job_when_pending_events_exist() {
        let _test_dir = ScopedTestDataDir::new("summary-with-pending-observation");
        let conn = db::open_db().expect("db should open");
        db::enqueue_pending(
            &conn,
            "claude-code",
            "sess-with-pending",
            "/tmp/remem",
            "Edit",
            Some(r#"{"file_path":"src/lib.rs"}"#),
            None,
            Some("/tmp/remem"),
        )
        .expect("pending observation should insert");

        enqueue_summary_jobs(
            &conn,
            "claude-code",
            "sess-with-pending",
            "/tmp/remem",
            r#"{"session_id":"sess-with-pending"}"#,
        )
        .expect("summary jobs should enqueue");

        let jobs = job_types(&conn);
        assert_eq!(
            jobs,
            vec![
                "observation".to_string(),
                "summary".to_string(),
                "compress".to_string()
            ]
        );
    }

    fn job_types(conn: &rusqlite::Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT job_type FROM jobs ORDER BY id ASC")
            .expect("job query should prepare");
        stmt.query_map([], |row| row.get(0))
            .expect("job query should run")
            .collect::<rusqlite::Result<Vec<String>>>()
            .expect("job rows should collect")
    }
}
