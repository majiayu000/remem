use std::time::Instant;

use anyhow::Result;

use crate::db;
use crate::hook_stdin::read_stdin_with_timeout;
use crate::perf::{format_phase_timings, push_elapsed, time_result, PhaseTiming};

use super::super::constants::SUMMARIZE_STDIN_TIMEOUT_MS;
use super::super::input::SummarizeInput;
use super::host::resolve_hook_host;
use super::spill::{replay_spilled_summary_hook_payloads, spill_summary_hook_payload};
use super::worker_launch::{spawn_worker_once_if_idle, WorkerSpawnDecision};

pub async fn summarize(host: Option<&str>, profile: Option<&str>) -> Result<()> {
    let Some(input) = read_stdin_with_timeout(SUMMARIZE_STDIN_TIMEOUT_MS)? else {
        return Ok(());
    };

    summarize_input(&input, host, profile).await
}

pub(super) async fn summarize_input(
    input: &str,
    host: Option<&str>,
    profile: Option<&str>,
) -> Result<()> {
    let total_start = Instant::now();
    let mut timings = Vec::new();
    let hook: SummarizeInput = match serde_json::from_str(input) {
        Ok(value) => value,
        Err(err) => {
            crate::log::warn(
                "summarize",
                &format!("invalid hook payload, skipping: {}", err),
            );
            return Ok(());
        }
    };
    if hook.session_id.is_none() {
        return Ok(());
    }
    let host = time_result(&mut timings, "resolve_host", || resolve_hook_host(host))?;
    let cwd = effective_cwd(&hook)?;
    let conn = match time_result(&mut timings, "open_db_for_hook", db::open_db_for_hook) {
        Ok(conn) => conn,
        Err(error) => {
            let spill_start = Instant::now();
            let spill_result =
                spill_summary_hook_payload(input, Some(&host), profile, Some(&cwd), &error);
            push_elapsed(&mut timings, "spill_payload", spill_start);
            let path = spill_result?;
            crate::log::error(
                "summarize",
                &format!(
                    "database open failed; spilled summary hook payload to {}: {}",
                    path.display(),
                    error
                ),
            );
            push_elapsed(&mut timings, "hook_total", total_start);
            log_summary_hook_timing("db_open_failed", &host, &timings);
            return Err(error);
        }
    };
    time_result(&mut timings, "enqueue_summary_payload", || {
        enqueue_summary_payload(&conn, input, Some(&host), profile)
    })?;
    if let Err(error) = time_result(&mut timings, "spill_replay", || {
        replay_spilled_summary_hook_payloads(&conn, |conn, record| {
            enqueue_summary_payload(
                conn,
                &record.input,
                record.host.as_deref(),
                record.profile.as_deref(),
            )
        })
    }) {
        crate::log::error(
            "summarize",
            &format!("summary hook spill replay failed; continuing with current payload: {error}"),
        );
    }
    match time_result(&mut timings, "worker_once_spawn", || {
        spawn_worker_once_if_idle(&conn)
    }) {
        Ok(WorkerSpawnDecision::Spawned) => {
            crate::log::info("summarize", "worker --once spawned");
        }
        Ok(WorkerSpawnDecision::SkippedHealthyWorker) => {
            crate::log::info("summarize", "worker heartbeat healthy; skip worker --once");
        }
        Ok(WorkerSpawnDecision::SkippedLaunchInProgress) => {
            crate::log::info(
                "summarize",
                "worker --once launch already in progress; skip spawn",
            );
        }
        Err(error) => {
            crate::log::error(
                "summarize",
                &format!("summary jobs queued but worker --once spawn failed: {error}"),
            );
        }
    }
    push_elapsed(&mut timings, "hook_total", total_start);
    log_summary_hook_timing("queued", &host, &timings);
    Ok(())
}

fn enqueue_summary_payload(
    conn: &rusqlite::Connection,
    input: &str,
    host: Option<&str>,
    profile: Option<&str>,
) -> Result<()> {
    let hook: SummarizeInput = serde_json::from_str(input)?;
    let Some(session_id) = &hook.session_id else {
        return Ok(());
    };
    let cwd = effective_cwd(&hook)?;
    let project = db::project_from_cwd(&cwd);
    let host = resolve_hook_host(host)?;
    let summary_payload = summary_payload_with_cwd(input, &cwd, profile)?;
    let compress_payload = compress_payload(profile)?;

    record_summary_capture_event(conn, &host, session_id, &project, &cwd, &summary_payload);
    enqueue_summary_jobs(
        conn,
        &host,
        session_id,
        &project,
        &summary_payload,
        &compress_payload,
    )?;
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
    db::maybe_enqueue_dream_job(
        conn,
        host,
        project,
        compress_input,
        300,
        crate::dream::DREAM_COOLDOWN_SECS,
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

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn log_summary_hook_timing(status: &str, host: &str, timings: &[PhaseTiming]) {
    crate::log::info(
        "summarize-perf",
        &format!(
            "status={} host={} timings=[{}]",
            status,
            host,
            format_phase_timings(timings)
        ),
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crate::db::{self, test_support::ScopedTestDataDir};

    use super::{
        compress_payload, enqueue_summary_jobs, record_summary_capture_event, resolve_hook_host,
        summarize_input, summary_payload_with_cwd,
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        old_values: Vec<(String, Option<String>)>,
    }

    impl EnvGuard {
        fn set(vars: &[(&str, Option<&str>)]) -> Self {
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

            Self { old_values }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.old_values.drain(..) {
                match value {
                    Some(value) => unsafe { std::env::set_var(&key, value) },
                    None => unsafe { std::env::remove_var(&key) },
                }
            }
        }
    }

    fn with_env_vars<T>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
        let Ok(_guard) = ENV_LOCK.lock() else {
            panic!("env lock should acquire");
        };
        let _env = EnvGuard::set(vars);
        f()
    }

    #[test]
    fn hook_host_normalizes_explicit_host() {
        with_env_vars(
            &[
                ("REMEM_SUMMARY_EXECUTOR", Some("claude-cli")),
                ("REMEM_EXECUTOR", Some("claude-cli")),
            ],
            || {
                assert!(matches!(
                    resolve_hook_host(Some("codex")).as_deref(),
                    Ok("codex-cli")
                ));
            },
        );
    }

    #[test]
    fn hook_host_uses_runtime_config_default() {
        let _test_dir = ScopedTestDataDir::new("summary-default-host");

        with_env_vars(
            &[
                ("REMEM_HOOK_HOST", None),
                ("REMEM_CONTEXT_HOST", None),
                ("REMEM_SUMMARY_EXECUTOR", None),
                ("REMEM_EXECUTOR", None),
            ],
            || {
                assert!(matches!(
                    resolve_hook_host(None).as_deref(),
                    Ok("codex-cli")
                ));
            },
        );
    }

    #[test]
    fn hook_host_preserves_legacy_summary_executor() {
        with_env_vars(
            &[
                ("REMEM_HOOK_HOST", None),
                ("REMEM_CONTEXT_HOST", None),
                ("REMEM_SUMMARY_EXECUTOR", Some("claude-cli")),
                ("REMEM_EXECUTOR", Some("codex-cli")),
            ],
            || {
                assert!(matches!(
                    resolve_hook_host(None).as_deref(),
                    Ok("claude-code")
                ));
            },
        );
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
    fn hosted_summary_hook_can_preserve_profile_override() -> anyhow::Result<()> {
        let host = resolve_hook_host(Some("codex"))?;
        let summary = summary_payload_with_cwd(
            r#"{"session_id":"sess-hosted-profile"}"#,
            "/tmp/project",
            Some("custom"),
        )?;
        let parsed: serde_json::Value = serde_json::from_str(&summary)?;

        assert_eq!(host, "codex-cli");
        assert_eq!(parsed["remem_ai_profile"].as_str(), Some("custom"));
        Ok(())
    }

    #[tokio::test]
    async fn summarize_hook_rejects_stale_schema_without_migrating() -> anyhow::Result<()> {
        let test_dir = ScopedTestDataDir::new("summary-hook-stale-schema");
        std::fs::create_dir_all(&test_dir.path)?;
        let setup = rusqlite::Connection::open(test_dir.db_path())?;
        setup.execute("CREATE TABLE marker (id INTEGER PRIMARY KEY)", [])?;
        drop(setup);
        let input = serde_json::json!({
            "session_id": "sess-summary-stale",
            "cwd": "/tmp/remem"
        })
        .to_string();

        let err = summarize_input(&input, Some("codex-cli"), None)
            .await
            .expect_err("stale hook database should fail closed");

        assert!(
            err.to_string().contains("hook database open requires"),
            "unexpected error: {err:#}"
        );
        let check = rusqlite::Connection::open(test_dir.db_path())?;
        let (migrations_exists, jobs_exists): (i64, i64) = check.query_row(
            "SELECT
                SUM(CASE WHEN name = '_schema_migrations' THEN 1 ELSE 0 END),
                SUM(CASE WHEN name = 'jobs' THEN 1 ELSE 0 END)
             FROM sqlite_master
             WHERE type = 'table'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(migrations_exists, 0);
        assert_eq!(jobs_exists, 0);
        assert!(super::super::spill::summary_spill_path().exists());
        Ok(())
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
        assert_eq!(
            jobs,
            vec![
                "summary".to_string(),
                "compress".to_string(),
                "dream".to_string()
            ]
        );
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
        assert_eq!(
            jobs,
            vec![
                "summary".to_string(),
                "compress".to_string(),
                "dream".to_string()
            ]
        );
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
        assert_eq!(
            jobs,
            vec![
                "summary".to_string(),
                "compress".to_string(),
                "dream".to_string()
            ]
        );
    }

    #[test]
    fn enqueue_summary_jobs_dedups_dream_and_preserves_profile_payload() {
        let _test_dir = ScopedTestDataDir::new("summary-dream-profile");
        let conn = db::open_db().expect("db should open");
        let payload = compress_payload(Some("custom")).expect("compress payload should serialize");

        enqueue_summary_jobs(
            &conn,
            "codex-cli",
            "sess-dream-a",
            "/tmp/remem",
            r#"{"session_id":"sess-dream-a"}"#,
            &payload,
        )
        .expect("first summary jobs should enqueue");
        enqueue_summary_jobs(
            &conn,
            "codex-cli",
            "sess-dream-b",
            "/tmp/remem",
            r#"{"session_id":"sess-dream-b"}"#,
            &payload,
        )
        .expect("second summary jobs should enqueue");

        let dream_payloads = job_payloads(&conn, "dream");
        assert_eq!(dream_payloads.len(), 1);
        let dream_payload: serde_json::Value =
            serde_json::from_str(&dream_payloads[0]).expect("dream payload should parse");
        assert_eq!(dream_payload["remem_ai_profile"].as_str(), Some("custom"));
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

    fn job_payloads(conn: &rusqlite::Connection, job_type: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM jobs WHERE job_type = ?1 ORDER BY id ASC")
            .expect("job query should prepare");
        stmt.query_map([job_type], |row| row.get(0))
            .expect("job query should run")
            .collect::<rusqlite::Result<Vec<String>>>()
            .expect("job rows should collect")
    }
}
