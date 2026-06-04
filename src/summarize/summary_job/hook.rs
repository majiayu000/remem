use anyhow::Result;

use crate::db;
use crate::hook_stdin::read_stdin_with_timeout;

use super::super::constants::SUMMARIZE_STDIN_TIMEOUT_MS;
use super::super::input::SummarizeInput;

pub async fn summarize(host: Option<&str>, profile: Option<&str>) -> Result<()> {
    if host.is_some() && profile.is_some() {
        anyhow::bail!("--host and --profile are mutually exclusive");
    }
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
    let host = resolve_hook_host(host)?;
    let conn = db::open_db()?;
    let summary_payload = summary_payload_with_cwd(&input, &cwd, profile)?;
    let compress_payload = compress_payload(profile)?;

    record_summary_capture_event(&conn, &host, session_id, &project, &cwd, &summary_payload);
    enqueue_summary_jobs(
        &conn,
        &host,
        session_id,
        &project,
        &summary_payload,
        &compress_payload,
    )?;
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

fn summary_payload_with_cwd(input: &str, cwd: &str, profile: Option<&str>) -> Result<String> {
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
    if let Some(profile) = clean_optional(profile) {
        obj.insert(
            crate::runtime_config::MEMORY_AI_PROFILE_FIELD.to_string(),
            serde_json::Value::String(profile),
        );
    }
    Ok(serde_json::to_string(&payload)?)
}

fn compress_payload(profile: Option<&str>) -> Result<String> {
    let mut payload = serde_json::Map::new();
    if let Some(profile) = clean_optional(profile) {
        payload.insert(
            crate::runtime_config::MEMORY_AI_PROFILE_FIELD.to_string(),
            serde_json::Value::String(profile),
        );
    }
    Ok(serde_json::to_string(&serde_json::Value::Object(payload))?)
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
    compress_input: &str,
) -> Result<()> {
    let ready_pending = db::count_pending_for_identity(conn, host, project, session_id)?;
    if ready_pending > 0 {
        crate::log::warn(
            "summarize",
            &format!(
                "ignored {ready_pending} legacy pending observation row(s); captures now use extraction_tasks"
            ),
        );
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
    db::enqueue_job(
        conn,
        host,
        db::JobType::Compress,
        project,
        None,
        compress_input,
        200,
    )?;
    crate::log::info(
        "summarize",
        &format!(
            "QUEUED summary session={} project={} legacy_pending_observations={}",
            session_id, project, ready_pending
        ),
    );
    Ok(())
}

fn resolve_hook_host(host: Option<&str>) -> Result<String> {
    if let Some(host) = clean_optional(host) {
        return Ok(crate::runtime_config::normalize_host(&host));
    }
    crate::runtime_config::default_host()
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

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use crate::db::{self, test_support::ScopedTestDataDir};

    use super::{
        compress_payload, enqueue_summary_jobs, record_summary_capture_event, resolve_hook_host,
        should_spawn_worker_once, stable_worker_dir, summary_payload_with_cwd,
    };

    #[test]
    fn hook_host_normalizes_explicit_host() {
        assert_eq!(resolve_hook_host(Some("codex")).unwrap(), "codex-cli");
    }

    #[test]
    fn hook_host_uses_runtime_config_default() {
        let _test_dir = ScopedTestDataDir::new("summary-default-host");

        assert_eq!(resolve_hook_host(None).unwrap(), "codex-cli");
    }

    #[test]
    fn summary_payload_with_cwd_fills_missing_cwd() {
        let payload =
            summary_payload_with_cwd(r#"{"session_id":"sess-cwd"}"#, "/tmp/project", None)
                .expect("payload should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&payload).expect("payload should parse");

        assert_eq!(parsed["session_id"].as_str(), Some("sess-cwd"));
        assert_eq!(parsed["cwd"].as_str(), Some("/tmp/project"));
    }

    #[test]
    fn summary_payload_with_cwd_preserves_existing_cwd() {
        let payload = summary_payload_with_cwd(
            r#"{"session_id":"sess-cwd","cwd":"/repo"}"#,
            "/tmp/project",
            None,
        )
        .expect("payload should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&payload).expect("payload should parse");

        assert_eq!(parsed["cwd"].as_str(), Some("/repo"));
    }

    #[test]
    fn summary_and_compress_payloads_preserve_profile() {
        let summary = summary_payload_with_cwd(
            r#"{"session_id":"sess-cwd"}"#,
            "/tmp/project",
            Some("custom"),
        )
        .expect("payload should serialize");
        let compress = compress_payload(Some("custom")).expect("payload should serialize");
        let summary: serde_json::Value =
            serde_json::from_str(&summary).expect("summary payload should parse");
        let compress: serde_json::Value =
            serde_json::from_str(&compress).expect("compress payload should parse");

        assert_eq!(summary["remem_ai_profile"].as_str(), Some("custom"));
        assert_eq!(compress["remem_ai_profile"].as_str(), Some("custom"));
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
            "{}",
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
            "{}",
        )
        .expect("summary jobs should enqueue");

        let jobs = job_types(&conn);
        assert_eq!(jobs, vec!["summary".to_string(), "compress".to_string()]);
    }

    #[test]
    fn enqueue_summary_jobs_ignores_legacy_pending_observations() {
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
            "{}",
        )
        .expect("summary jobs should enqueue");

        let jobs = job_types(&conn);
        assert_eq!(jobs, vec!["summary".to_string(), "compress".to_string()]);
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
