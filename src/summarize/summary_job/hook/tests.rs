use std::sync::Mutex;

use crate::db::{self, test_support::ScopedTestDataDir};

use super::{
    record_summary_capture_event, resolve_hook_host, summarize_input, summary_payload_with_cwd,
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
    let payload = summary_payload_with_cwd(r#"{"session_id":"sess-cwd"}"#, "/tmp/project", None)
        .expect("payload should serialize");
    let parsed: serde_json::Value = serde_json::from_str(&payload).expect("payload should parse");

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
    let parsed: serde_json::Value = serde_json::from_str(&payload).expect("payload should parse");

    assert_eq!(parsed["cwd"].as_str(), Some("/repo"));
}

#[test]
fn summary_payload_preserves_profile() {
    let summary = summary_payload_with_cwd(
        r#"{"session_id":"sess-cwd"}"#,
        "/tmp/project",
        Some("custom"),
    )
    .expect("payload should serialize");
    let summary: serde_json::Value =
        serde_json::from_str(&summary).expect("summary payload should parse");

    assert_eq!(summary["remem_ai_profile"].as_str(), Some("custom"));
}

#[test]
fn summary_payload_snapshots_transcript_byte_length() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("summary-transcript-byte-length");
    std::fs::create_dir_all(&test_dir.path)?;
    let transcript = test_dir.path.join("transcript.jsonl");
    std::fs::write(&transcript, "first line\nsecond line\n")?;
    let input = serde_json::json!({
        "session_id": "sess-transcript-byte-length",
        "transcript_path": transcript
    })
    .to_string();

    let payload = summary_payload_with_cwd(&input, "/tmp/project", None)?;
    let parsed: serde_json::Value = serde_json::from_str(&payload)?;

    assert_eq!(
        parsed["transcript_byte_len"].as_u64(),
        Some(std::fs::metadata(&transcript)?.len())
    );
    Ok(())
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

#[tokio::test]
async fn summarize_hook_spills_when_capture_ledger_fails() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("summary-hook-capture-failure");
    drop(db::open_db()?);
    match std::fs::remove_file(super::super::spill::summary_spill_path()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let input = serde_json::json!({
        "session_id": "sess-summary-capture-failure",
        "cwd": "/tmp/remem"
    })
    .to_string();

    let err = summarize_input(&input, Some("unknown"), None)
        .await
        .expect_err("capture ledger failure should fail closed");

    assert!(
        err.to_string().contains("invalid capture host"),
        "unexpected error: {err:#}"
    );
    let conn = db::open_db()?;
    let job_count: i64 = conn.query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))?;
    assert_eq!(job_count, 0);
    assert!(super::super::spill::summary_spill_path().exists());
    Ok(())
}

#[tokio::test]
async fn summarize_hook_spills_when_transcript_snapshot_fails() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("summary-hook-transcript-snapshot-failure");
    drop(db::open_db()?);
    let missing_transcript = test_dir.path.join("missing-transcript.jsonl");
    let input = serde_json::json!({
        "session_id": "sess-summary-transcript-snapshot-failure",
        "cwd": "/tmp/remem",
        "transcript_path": missing_transcript
    })
    .to_string();

    let result = summarize_input(&input, Some("codex-cli"), None).await;
    let error = match result {
        Ok(()) => anyhow::bail!("missing transcript snapshot unexpectedly succeeded"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("snapshot transcript length"));
    let conn = db::open_db()?;
    let captured_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events
             WHERE session_id = 'sess-summary-transcript-snapshot-failure'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(captured_events, 0);
    assert!(super::super::spill::summary_spill_path().exists());
    Ok(())
}

#[tokio::test]
async fn summarize_hook_runs_stop_side_effects_without_summary_job() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("summary-hook-side-effects");
    let conn = db::open_db()?;
    let now = chrono::Utc::now().timestamp();
    db::upsert_worker_heartbeat(
        &conn,
        "worker-daemon",
        i64::from(std::process::id()),
        now,
        now,
    )?;
    drop(conn);
    let input = serde_json::json!({
        "session_id": "sess-summary-hook-side-effects",
        "cwd": "/tmp/remem",
        "last_assistant_message": "hook side effect assistant message"
    })
    .to_string();

    summarize_input(&input, Some("codex-cli"), None).await?;

    let conn = db::open_db()?;
    let captured_events: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_events
             WHERE session_id = 'sess-summary-hook-side-effects'
               AND event_type = 'session_stop'",
        [],
        |row| row.get(0),
    )?;
    let job_count: i64 = conn.query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))?;
    let summary_jobs: i64 = conn.query_row(
        "SELECT COUNT(*) FROM jobs WHERE job_type = 'summary'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(captured_events, 1);
    assert_eq!(job_count, 0);
    assert_eq!(summary_jobs, 0);
    Ok(())
}

#[tokio::test]
async fn summarize_hook_replays_same_session_spill_for_different_project() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("summary-hook-project-scoped-spill");
    let conn = db::open_db()?;
    let now = chrono::Utc::now().timestamp();
    db::upsert_worker_heartbeat(
        &conn,
        &db::current_worker_owner("daemon", std::process::id(), now * 1000),
        i64::from(std::process::id()),
        now,
        now,
    )?;
    let old_transcript = test_dir.path.join("old-other-transcript.jsonl");
    let current_transcript = test_dir.path.join("current-transcript.jsonl");
    std::fs::write(
        &old_transcript,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"old"}]}}"#,
    )?;
    std::fs::write(
        &current_transcript,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"current"}]}}"#,
    )?;
    let old_input = serde_json::json!({
        "session_id": "sess-summary-shared-id",
        "cwd": "/tmp/remem-other",
        "transcript_path": old_transcript
    })
    .to_string();
    super::super::spill::spill_summary_hook_payload(
        &old_input,
        Some("codex-cli"),
        None,
        Some("/tmp/remem-other"),
        &anyhow::anyhow!("stale db"),
    )?;

    let current_input = serde_json::json!({
        "session_id": "sess-summary-shared-id",
        "cwd": "/tmp/remem-current",
        "transcript_path": current_transcript
    })
    .to_string();
    summarize_input(&current_input, Some("codex-cli"), None).await?;

    let event_count: i64 = conn.query_row(
        "SELECT COUNT(*)
             FROM captured_events
             WHERE event_type = 'session_stop'
               AND session_id = 'sess-summary-shared-id'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(event_count, 2);
    assert!(!super::super::spill::summary_spill_path().exists());
    Ok(())
}

#[test]
fn capture_ledger_failure_blocks_followup_jobs() {
    let _test_dir = ScopedTestDataDir::new("summary-legacy-unknown-host");
    let conn = db::open_db().expect("db should open");

    let err = record_summary_capture_event(
        &conn,
        "unknown",
        "sess-legacy",
        "/tmp/remem",
        "/tmp/remem",
        r#"{"session_id":"sess-legacy","cwd":"/tmp/remem"}"#,
        None,
        &[],
    )
    .expect_err("capture ledger failure should stop summary hook followups");

    assert!(err.to_string().contains("invalid capture host"));
    let job_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
        .expect("job count should query");
    assert_eq!(job_count, 0);
}
