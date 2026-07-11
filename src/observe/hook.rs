use super::filter::{event_skip_reason, skip_detail};
use super::native::sync_native_memory;
use super::spill::{
    record_capture_drop_lossy, replay_spilled_capture_events,
    spill_capture_event_with_git_evidence, SPILL_REASON_CAPTURE_PERSISTENCE_FAILED,
    SPILL_REASON_DB_OPEN_FAILED,
};
use crate::db;
use anyhow::{Context, Result};

pub async fn observe(host: Option<&str>) -> Result<()> {
    let input = std::io::read_to_string(std::io::stdin())?;
    observe_input(&input, host).await
}

pub(super) async fn observe_input(input: &str, host: Option<&str>) -> Result<()> {
    let Some((adapter, event)) = detect_adapter_for_host(input, host) else {
        record_capture_drop_lossy(
            host.map(crate::runtime_config::normalize_host).as_deref(),
            None,
            "adapter_mismatch",
            Some("no capture adapter matched hook input"),
        );
        return Ok(());
    };
    let capture_host = host
        .map(crate::runtime_config::normalize_host)
        .unwrap_or_else(|| adapter.name().to_string());
    let Some(summary) = adapter.classify_event(&event) else {
        record_capture_drop_lossy(
            Some(&capture_host),
            Some(&event),
            "unclassified_event",
            Some("adapter did not produce a capture summary"),
        );
        return Ok(());
    };
    let git_evidence =
        crate::git_evidence::from_observed_event(&event, &summary).with_context(|| {
            format!(
                "capture explicit commit evidence host={} session={} project={}",
                capture_host, event.session_id, event.project
            )
        })?;
    if let Some(reason) = event_skip_reason(adapter, &event, !git_evidence.is_empty()) {
        let detail = skip_detail(&event);
        record_capture_drop_lossy(Some(&capture_host), Some(&event), reason, detail.as_deref());
        return Ok(());
    }

    let content = capture_event_content_with_git_evidence(&event, &summary, &git_evidence);
    let event_id = db::unique_capture_event_id("tool_result", &content);
    let conn = match db::open_db_for_hook() {
        Ok(conn) => conn,
        Err(error) => {
            let path = spill_capture_event_with_git_evidence(
                &capture_host,
                &event_id,
                &event,
                &summary,
                &git_evidence,
                SPILL_REASON_DB_OPEN_FAILED,
                &error,
            )?;
            crate::log::error(
                "observe",
                &format!(
                    "database open failed; spilled capture event to {}: {}",
                    path.display(),
                    error
                ),
            );
            return Err(error);
        }
    };
    replay_spilled_capture_events(&conn)?;
    if let Err(error) = record_live_observed_event_with_id(
        &conn,
        &capture_host,
        &event_id,
        &event,
        &summary,
        &git_evidence,
    ) {
        let path = spill_capture_event_with_git_evidence(
            &capture_host,
            &event_id,
            &event,
            &summary,
            &git_evidence,
            SPILL_REASON_CAPTURE_PERSISTENCE_FAILED,
            &error,
        )?;
        let spill_path = path.display().to_string();
        if let Err(drop_error) = crate::db::record_capture_drop(
            &conn,
            &crate::db::CaptureDropInput {
                host: Some(&capture_host),
                session_id: Some(&event.session_id),
                project: Some(&event.project),
                tool_name: Some(&event.tool_name),
                reason: SPILL_REASON_CAPTURE_PERSISTENCE_FAILED,
                detail: Some(&error.to_string()),
                spill_path: Some(&spill_path),
                recovered_event_id: None,
            },
        ) {
            crate::log::warn(
                "observe",
                &format!("capture persistence drop ledger write failed: {drop_error}"),
            );
        }
        crate::log::error(
            "observe",
            &format!(
                "capture persistence failed; spilled capture event to {}: {}",
                path.display(),
                error
            ),
        );
        return Err(error);
    }

    Ok(())
}

pub(super) fn record_observed_event_with_id(
    conn: &rusqlite::Connection,
    capture_host: &str,
    event_id: &str,
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
    git_evidence: &[crate::git_util::GitCommitEvidence],
) -> Result<i64> {
    record_observed_event(conn, capture_host, event_id, event, summary, git_evidence)
}

fn record_live_observed_event_with_id(
    conn: &rusqlite::Connection,
    capture_host: &str,
    event_id: &str,
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
    git_evidence: &[crate::git_util::GitCommitEvidence],
) -> Result<i64> {
    record_observed_event(conn, capture_host, event_id, event, summary, git_evidence)
}

fn record_observed_event(
    conn: &rusqlite::Connection,
    capture_host: &str,
    event_id: &str,
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
    git_evidence: &[crate::git_util::GitCommitEvidence],
) -> Result<i64> {
    let capture_event_id = record_capture_event_with_git_evidence(
        conn,
        capture_host,
        event_id,
        event,
        summary,
        git_evidence,
    )?;
    crate::memory::insert_event(
        conn,
        &event.session_id,
        &event.project,
        &summary.event_type,
        &summary.summary,
        summary.detail.as_deref(),
        summary.files_json.as_deref(),
        summary.exit_code,
    )?;

    crate::log::info(
        "observe",
        &format!(
            "EVENT {} project={} tool={}",
            summary.summary, event.project, event.tool_name
        ),
    );

    if matches!(event.tool_name.as_str(), "Write" | "Edit") {
        let branch = git_evidence
            .first()
            .and_then(|evidence| evidence.metadata.branch.clone())
            .or_else(|| event.cwd.as_deref().and_then(db::detect_git_branch));
        if let Some(file_path) = event
            .tool_input
            .as_ref()
            .and_then(|value| value["file_path"].as_str())
        {
            if let Err(error) =
                sync_native_memory(conn, &event.session_id, file_path, branch.as_deref())
            {
                crate::log::warn("observe", &format!("native memory sync failed: {}", error));
            }
        }
    }

    Ok(capture_event_id)
}

pub(super) fn detect_adapter_for_host(
    input: &str,
    host: Option<&str>,
) -> Option<(
    &'static dyn crate::adapter::ToolAdapter,
    crate::adapter::ParsedHookEvent,
)> {
    let Some(host) = host else {
        return crate::adapter::detect_adapter(input);
    };
    let adapter_name = crate::runtime_config::resolve_host_runtime_config(Some(host))
        .map(|config| config.capture_adapter)
        .unwrap_or_else(|_| crate::runtime_config::normalize_host(host));
    crate::adapter::detect_adapter_by_name(input, &adapter_name)
        .or_else(|| crate::adapter::detect_adapter(input))
}

#[cfg(test)]
fn record_capture_event_with_id(
    conn: &rusqlite::Connection,
    host: &str,
    event_id: &str,
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
) -> Result<i64> {
    record_capture_event_with_git_evidence(conn, host, event_id, event, summary, &[])
}

fn record_capture_event_with_git_evidence(
    conn: &rusqlite::Connection,
    host: &str,
    event_id: &str,
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
    git_evidence: &[crate::git_util::GitCommitEvidence],
) -> Result<i64> {
    let content = capture_event_content_with_git_evidence(event, summary, git_evidence);
    let outcome = db::record_captured_event_with_id_and_reference_time_and_git_evidence(
        conn,
        &db::CaptureEventInput {
            host,
            session_id: &event.session_id,
            project: &event.project,
            cwd: event.cwd.as_deref(),
            event_type: "tool_result",
            role: None,
            tool_name: Some(&event.tool_name),
            content: &content,
            task_kind: Some(db::ExtractionTaskKind::ObservationExtract),
        },
        Some(event_id),
        event.reference_time_epoch,
        git_evidence,
    )?;
    Ok(outcome.event_row_id)
}

#[cfg(test)]
fn capture_event_content(
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
) -> String {
    capture_event_content_with_git_evidence(event, summary, &[])
}

fn capture_event_content_with_git_evidence(
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
    git_evidence: &[crate::git_util::GitCommitEvidence],
) -> String {
    let git_branch = git_evidence
        .first()
        .and_then(|evidence| evidence.metadata.branch.as_deref());
    serde_json::json!({
        "summary": &summary.summary,
        "event_type": &summary.event_type,
        "detail": summary.detail.as_deref(),
        "files": summary.files_json.as_deref(),
        "exit_code": summary.exit_code,
        "tool_name": &event.tool_name,
        "tool_input": event
            .tool_input
            .as_ref()
            .map(crate::adapter::common::redact_sensitive_value),
        "tool_response": event
            .tool_response
            .as_ref()
            .map(crate::adapter::common::redact_sensitive_value),
        "git_branch": git_branch,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crate::adapter::{codex::CodexAdapter, EventSummary, ParsedHookEvent};
    use crate::db::{self, test_support::ScopedTestDataDir};

    use super::super::filter::{event_skip_reason, skip_detail};
    use super::super::spill::spill_capture_event;
    use super::{
        capture_event_content, observe_input, record_capture_event_with_id,
        SPILL_REASON_CAPTURE_PERSISTENCE_FAILED, SPILL_REASON_DB_OPEN_FAILED,
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn codex_bash_event() -> ParsedHookEvent {
        ParsedHookEvent {
            session_id: "session".to_string(),
            cwd: Some("/tmp".to_string()),
            project: "/tmp".to_string(),
            reference_time_epoch: None,
            tool_name: "Bash".to_string(),
            tool_input: Some(serde_json::json!({ "command": "cargo test" })),
            tool_response: Some(serde_json::json!({ "exitCode": 0 })),
        }
    }

    #[test]
    fn codex_bash_observe_skips_by_default() {
        let _guard = ENV_LOCK.lock().expect("env lock should acquire");
        unsafe { std::env::remove_var("REMEM_ENABLE_CODEX_BASH_OBSERVE") };

        assert_eq!(
            event_skip_reason(&CodexAdapter, &codex_bash_event(), false),
            Some("codex_bash_disabled")
        );
    }

    #[tokio::test]
    async fn observe_records_codex_bash_skip_in_drop_ledger() -> anyhow::Result<()> {
        let _guard = ENV_LOCK.lock().expect("env lock should acquire");
        unsafe { std::env::remove_var("REMEM_ENABLE_CODEX_BASH_OBSERVE") };
        let _test_dir = ScopedTestDataDir::new("observe-codex-bash-drop");
        let setup = db::open_db()?;
        drop(setup);
        let input = serde_json::json!({
            "session_id": "sess-bash-skip",
            "cwd": "/tmp/remem",
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": { "command": "cargo test" },
            "tool_result": { "exitCode": 0 }
        })
        .to_string();

        observe_input(&input, Some("codex-cli")).await?;

        let conn = db::open_db()?;
        let reason: String = conn.query_row(
            "SELECT reason FROM capture_drop_events WHERE session_id = ?1",
            ["sess-bash-skip"],
            |row| row.get(0),
        )?;
        assert_eq!(reason, "codex_bash_disabled");
        Ok(())
    }

    #[tokio::test]
    async fn observe_spills_when_hook_db_open_would_need_migration() -> anyhow::Result<()> {
        let test_dir = ScopedTestDataDir::new("observe-hook-open-stale-schema");
        std::fs::create_dir_all(&test_dir.path)?;
        let setup = rusqlite::Connection::open(test_dir.db_path())?;
        setup.execute("CREATE TABLE marker (id INTEGER PRIMARY KEY)", [])?;
        drop(setup);
        let input = serde_json::json!({
            "session_id": "sess-stale-hook-db",
            "cwd": "/tmp/remem",
            "tool_name": "Edit",
            "tool_input": {"file_path": "src/lib.rs"},
            "tool_response": {"content": "edited"}
        })
        .to_string();

        let err = observe_input(&input, Some("claude-code"))
            .await
            .expect_err("stale hook database should fail closed");

        assert!(
            err.to_string().contains("hook database open requires"),
            "unexpected error: {err:#}"
        );
        assert!(crate::db::data_dir().join("capture-spill.jsonl").exists());
        let check = rusqlite::Connection::open(test_dir.db_path())?;
        let migrations_exists: i64 = check.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_schema_migrations'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(migrations_exists, 0);
        let spill = std::fs::read_to_string(crate::db::data_dir().join("capture-spill.jsonl"))?;
        assert!(spill.contains(SPILL_REASON_DB_OPEN_FAILED));
        Ok(())
    }

    #[tokio::test]
    async fn observe_skip_drop_does_not_migrate_stale_database() -> anyhow::Result<()> {
        let _guard = ENV_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("env lock poisoned"))?;
        unsafe { std::env::remove_var("REMEM_ENABLE_CODEX_BASH_OBSERVE") };
        let test_dir = ScopedTestDataDir::new("observe-skip-stale-schema");
        std::fs::create_dir_all(&test_dir.path)?;
        let setup = rusqlite::Connection::open(test_dir.db_path())?;
        setup.execute("CREATE TABLE marker (id INTEGER PRIMARY KEY)", [])?;
        drop(setup);
        let input = serde_json::json!({
            "session_id": "sess-skip-stale",
            "cwd": "/tmp/remem",
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": { "command": "cargo test" },
            "tool_result": { "exitCode": 0 }
        })
        .to_string();

        observe_input(&input, Some("codex-cli")).await?;

        let check = rusqlite::Connection::open(test_dir.db_path())?;
        let migrations_exists: i64 = check.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_schema_migrations'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(migrations_exists, 0);
        Ok(())
    }

    #[test]
    fn codex_bash_observe_can_be_enabled_explicitly() {
        let _guard = ENV_LOCK.lock().expect("env lock should acquire");
        unsafe { std::env::set_var("REMEM_ENABLE_CODEX_BASH_OBSERVE", "1") };
        let event = codex_bash_event();
        let skipped = event_skip_reason(&CodexAdapter, &event, false);
        unsafe { std::env::remove_var("REMEM_ENABLE_CODEX_BASH_OBSERVE") };

        assert_eq!(skipped, None);
    }

    #[test]
    fn skip_detail_redacts_command_before_truncating() -> anyhow::Result<()> {
        let command = "echo api_key=short-secret".to_string();
        let event = ParsedHookEvent {
            session_id: "session".to_string(),
            cwd: Some("/tmp".to_string()),
            project: "/tmp".to_string(),
            reference_time_epoch: None,
            tool_name: "Bash".to_string(),
            tool_input: Some(serde_json::json!({ "command": command })),
            tool_response: Some(serde_json::json!({ "exitCode": 0 })),
        };

        let detail =
            skip_detail(&event).ok_or_else(|| anyhow::anyhow!("bash command detail missing"))?;

        assert!(
            !detail.contains("short-secret"),
            "raw token fragment leaked: {detail}"
        );
        assert!(detail.contains("[REDACTED]"));
        Ok(())
    }

    #[test]
    fn skip_detail_does_not_leak_secret_at_truncation_boundary() -> anyhow::Result<()> {
        let command = format!("echo {} api_key=short-secret", "x".repeat(222));
        let event = ParsedHookEvent {
            session_id: "session".to_string(),
            cwd: Some("/tmp".to_string()),
            project: "/tmp".to_string(),
            reference_time_epoch: None,
            tool_name: "Bash".to_string(),
            tool_input: Some(serde_json::json!({ "command": command })),
            tool_response: Some(serde_json::json!({ "exitCode": 0 })),
        };

        let detail =
            skip_detail(&event).ok_or_else(|| anyhow::anyhow!("bash command detail missing"))?;

        assert!(
            !detail.contains("short-secret"),
            "raw token fragment leaked: {detail}"
        );
        Ok(())
    }

    #[test]
    fn record_capture_event_writes_ledger_and_coalesced_task() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("observe-capture-ledger");
        let conn = db::open_db()?;
        let event = ParsedHookEvent {
            session_id: "sess-observe".to_string(),
            cwd: Some("/tmp/remem".to_string()),
            project: "/tmp/remem".to_string(),
            reference_time_epoch: Some(1_600_000_000),
            tool_name: "Edit".to_string(),
            tool_input: Some(serde_json::json!({ "file_path": "src/lib.rs" })),
            tool_response: None,
        };
        let summary = EventSummary {
            event_type: "file_edit".to_string(),
            summary: "Edit src/lib.rs".to_string(),
            detail: None,
            files_json: Some(r#"["src/lib.rs"]"#.to_string()),
            exit_code: None,
        };

        let event_id =
            db::unique_capture_event_id("tool_result", &capture_event_content(&event, &summary));
        record_capture_event_with_id(&conn, "claude-code", &event_id, &event, &summary)?;
        let mut retry_event = event.clone();
        retry_event.reference_time_epoch = None;
        record_capture_event_with_id(&conn, "claude-code", &event_id, &retry_event, &summary)?;

        let captured_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))?;
        let task_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM extraction_tasks", [], |row| {
                row.get(0)
            })?;
        assert_eq!(captured_count, 1);
        assert_eq!(task_count, 1);
        let (created_at, reference_time, inserted_at): (i64, i64, i64) = conn.query_row(
            "SELECT created_at_epoch, reference_time_epoch, inserted_at_epoch
                 FROM captured_events
                 WHERE session_id = 'sess-observe'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(created_at, 1_600_000_000);
        assert_eq!(reference_time, 1_600_000_000);
        assert!(inserted_at >= reference_time);
        Ok(())
    }

    #[tokio::test]
    async fn observe_writes_only_normalized_capture_pipeline() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("observe-normalized-only");
        let setup = db::open_db()?;
        drop(setup);
        let project_dir = std::env::temp_dir().join(format!(
            "remem-observe-project-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&project_dir)?;
        let input = serde_json::json!({
            "session_id": "sess-normalized-observe",
            "cwd": project_dir,
            "tool_name": "Edit",
            "tool_input": {"file_path": "src/lib.rs"},
            "tool_response": {"content": "edited"}
        })
        .to_string();

        let test_result = async {
            observe_input(&input, Some("claude-code")).await?;

            let conn = db::open_db()?;
            let captured_count: i64 =
                conn.query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))?;
            let extraction_count: i64 =
                conn.query_row("SELECT COUNT(*) FROM extraction_tasks", [], |row| {
                    row.get(0)
                })?;
            let pending_count: i64 =
                conn.query_row("SELECT COUNT(*) FROM pending_observations", [], |row| {
                    row.get(0)
                })?;
            let observation_jobs: i64 = conn.query_row(
                "SELECT COUNT(*) FROM jobs WHERE job_type = 'observation'",
                [],
                |row| row.get(0),
            )?;

            anyhow::ensure!(captured_count == 1, "expected one captured event");
            anyhow::ensure!(extraction_count == 1, "expected one extraction task");
            anyhow::ensure!(pending_count == 0, "legacy pending queue must stay empty");
            anyhow::ensure!(
                observation_jobs == 0,
                "legacy observation jobs must stay empty"
            );
            Ok::<(), anyhow::Error>(())
        }
        .await;

        let _ = std::fs::remove_dir_all(&project_dir);
        test_result
    }

    #[tokio::test]
    async fn observe_appends_legacy_events_for_identical_normal_events() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("observe-identical-normal-events");
        let setup = db::open_db()?;
        drop(setup);
        let project_dir = std::env::temp_dir().join(format!(
            "remem-observe-duplicates-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&project_dir)?;
        let input = serde_json::json!({
            "session_id": "sess-identical-normal",
            "cwd": project_dir,
            "tool_name": "Edit",
            "tool_input": {"file_path": "src/lib.rs"},
            "tool_response": {"content": "edited"}
        })
        .to_string();

        let test_result = async {
            observe_input(&input, Some("claude-code")).await?;
            observe_input(&input, Some("claude-code")).await?;

            let conn = db::open_db()?;
            let captured_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM captured_events WHERE session_id = 'sess-identical-normal'",
                [],
                |row| row.get(0),
            )?;
            let legacy_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM events WHERE session_id = 'sess-identical-normal'",
                [],
                |row| row.get(0),
            )?;

            anyhow::ensure!(captured_count == 2, "expected two captured events");
            anyhow::ensure!(legacy_count == 2, "expected two legacy events");
            Ok::<(), anyhow::Error>(())
        }
        .await;

        std::fs::remove_dir_all(&project_dir)?;
        test_result
    }

    #[tokio::test]
    async fn observe_spills_persistence_failure_and_replays_capture_once() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("observe-persist-failure-spill");
        let failing_input = serde_json::json!({
            "session_id": "sess-persist-fail",
            "cwd": "/tmp/remem",
            "tool_name": "Edit",
            "tool_input": {"file_path": "src/lib.rs"},
            "tool_response": {"content": "edited"}
        })
        .to_string();

        let conn = db::open_db()?;
        conn.execute_batch(
            "CREATE TRIGGER fail_events_insert
             BEFORE INSERT ON events
             BEGIN
                 SELECT RAISE(FAIL, 'events blocked');
             END;",
        )?;
        drop(conn);

        let err = observe_input(&failing_input, Some("claude-code"))
            .await
            .expect_err("event insert failure should spill");
        assert!(err.to_string().contains("events blocked"), "{err}");
        assert!(crate::db::data_dir().join("capture-spill.jsonl").exists());

        let conn = db::open_db()?;
        let partial_captures: i64 = conn.query_row(
            "SELECT COUNT(*) FROM captured_events WHERE session_id = 'sess-persist-fail'",
            [],
            |row| row.get(0),
        )?;
        let partial_events: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE session_id = 'sess-persist-fail'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(partial_captures, 1);
        assert_eq!(partial_events, 0);
        let partial_drop: (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COUNT(recovered_event_id)
             FROM capture_drop_events
             WHERE session_id = 'sess-persist-fail'
               AND reason = 'capture_persistence_failed'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let partial_stats = db::query_system_stats(&conn)?;
        assert_eq!(partial_drop, (1, 0));
        assert_eq!(partial_stats.actionable_capture_drops, 1);
        assert_eq!(partial_stats.unrecovered_capture_spills, 1);
        conn.execute_batch("DROP TRIGGER fail_events_insert;")?;
        drop(conn);

        let replay_trigger = serde_json::json!({
            "session_id": "sess-replay-trigger",
            "cwd": "/tmp/remem",
            "tool_name": "Edit",
            "tool_input": {"file_path": "src/other.rs"},
            "tool_response": {"content": "edited"}
        })
        .to_string();
        observe_input(&replay_trigger, Some("claude-code")).await?;

        let conn = db::open_db()?;
        let replayed_captures: i64 = conn.query_row(
            "SELECT COUNT(*) FROM captured_events WHERE session_id = 'sess-persist-fail'",
            [],
            |row| row.get(0),
        )?;
        let replayed_events: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE session_id = 'sess-persist-fail'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(replayed_captures, 1);
        assert_eq!(replayed_events, 1);
        let replayed_drop: (String, Option<i64>) = conn.query_row(
            "SELECT reason, recovered_event_id
             FROM capture_drop_events
             WHERE session_id = 'sess-persist-fail'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let replayed_stats = db::query_system_stats(&conn)?;
        assert_eq!(replayed_drop.0, SPILL_REASON_CAPTURE_PERSISTENCE_FAILED);
        assert!(replayed_drop.1.is_some());
        assert_eq!(replayed_stats.actionable_capture_drops, 0);
        assert_eq!(replayed_stats.unrecovered_capture_spills, 0);
        assert!(!crate::db::data_dir().join("capture-spill.jsonl").exists());

        let replayed_event_id: String = conn.query_row(
            "SELECT event_id FROM captured_events WHERE session_id = 'sess-persist-fail'",
            [],
            |row| row.get(0),
        )?;
        let replayed_summary = conn.query_row(
            "SELECT event_type, summary, detail, files, exit_code
             FROM events
             WHERE session_id = 'sess-persist-fail'",
            [],
            |row| {
                Ok(EventSummary {
                    event_type: row.get(0)?,
                    summary: row.get(1)?,
                    detail: row.get(2)?,
                    files_json: row.get(3)?,
                    exit_code: row.get(4)?,
                })
            },
        )?;
        drop(conn);

        let replayed_event = ParsedHookEvent {
            session_id: "sess-persist-fail".to_string(),
            cwd: Some("/tmp/remem".to_string()),
            project: "/tmp/remem".to_string(),
            reference_time_epoch: None,
            tool_name: "Edit".to_string(),
            tool_input: Some(serde_json::json!({"file_path": "src/lib.rs"})),
            tool_response: Some(serde_json::json!({"content": "edited"})),
        };
        spill_capture_event(
            "claude-code",
            &replayed_event_id,
            &replayed_event,
            &replayed_summary,
            SPILL_REASON_CAPTURE_PERSISTENCE_FAILED,
            &anyhow::anyhow!("retry same partial capture"),
        )?;
        observe_input(&replay_trigger, Some("claude-code")).await?;

        let conn = db::open_db()?;
        let retry_replayed_events: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE session_id = 'sess-persist-fail'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(retry_replayed_events, 1);
        Ok(())
    }
}
