use anyhow::Result;
use rusqlite::OptionalExtension;

use crate::db;

use super::native::sync_native_memory;
use super::spill::{
    record_capture_drop_lossy, replay_spilled_capture_events, spill_capture_event,
    SPILL_REASON_CAPTURE_PERSISTENCE_FAILED, SPILL_REASON_DB_OPEN_FAILED,
};

pub async fn session_init(host: Option<&str>) -> Result<()> {
    let timer = crate::log::Timer::start("session-init", "");
    let input = std::io::read_to_string(std::io::stdin())?;
    crate::log::debug(
        "session-init",
        &format!("raw input: {}", crate::db::truncate_str(&input, 500)),
    );

    let Some(event) = session_init_event(&input, host) else {
        timer.done("skipped");
        return Ok(());
    };

    let project = event.project.clone();
    let conn = db::open_db()?;
    db::upsert_session(&conn, &event.session_id, &event.project, None)?;

    timer.done(&format!("project={}", project));
    Ok(())
}

fn session_init_event(input: &str, host: Option<&str>) -> Option<crate::adapter::ParsedHookEvent> {
    let Some((_adapter, event)) = detect_adapter_for_host(input, host) else {
        crate::log::warn("session-init", "SKIP no adapter matched hook input");
        return None;
    };

    crate::log::info(
        "session-init",
        &format!("project={} session={}", event.project, event.session_id),
    );
    Some(event)
}

pub async fn observe(host: Option<&str>) -> Result<()> {
    let input = std::io::read_to_string(std::io::stdin())?;
    observe_input(&input, host).await
}

async fn observe_input(input: &str, host: Option<&str>) -> Result<()> {
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
    if let Some(reason) = event_skip_reason(adapter, &event) {
        record_capture_drop_lossy(
            Some(&capture_host),
            Some(&event),
            reason,
            skip_detail(&event),
        );
        return Ok(());
    }

    let Some(summary) = adapter.classify_event(&event) else {
        record_capture_drop_lossy(
            Some(&capture_host),
            Some(&event),
            "unclassified_event",
            Some("adapter did not produce a capture summary"),
        );
        return Ok(());
    };

    let content = capture_event_content(&event, &summary);
    let event_id = db::unique_capture_event_id("tool_result", &content);
    let conn = match db::open_db() {
        Ok(conn) => conn,
        Err(error) => {
            let path = spill_capture_event(
                &capture_host,
                &event_id,
                &event,
                &summary,
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
    if let Err(error) =
        record_live_observed_event_with_id(&conn, &capture_host, &event_id, &event, &summary)
    {
        let path = spill_capture_event(
            &capture_host,
            &event_id,
            &event,
            &summary,
            SPILL_REASON_CAPTURE_PERSISTENCE_FAILED,
            &error,
        )?;
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
) -> Result<i64> {
    record_observed_event(
        conn,
        capture_host,
        event_id,
        event,
        summary,
        LegacyEventInsert::Deduplicate,
    )
}

fn record_live_observed_event_with_id(
    conn: &rusqlite::Connection,
    capture_host: &str,
    event_id: &str,
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
) -> Result<i64> {
    record_observed_event(
        conn,
        capture_host,
        event_id,
        event,
        summary,
        LegacyEventInsert::Append,
    )
}

enum LegacyEventInsert {
    Append,
    Deduplicate,
}

fn record_observed_event(
    conn: &rusqlite::Connection,
    capture_host: &str,
    event_id: &str,
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
    legacy_insert: LegacyEventInsert,
) -> Result<i64> {
    let capture_event_id =
        record_capture_event_with_id(conn, capture_host, event_id, event, summary)?;
    match legacy_insert {
        LegacyEventInsert::Append => {
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
        }
        LegacyEventInsert::Deduplicate => insert_memory_event_once(conn, event, summary)?,
    }

    crate::log::info(
        "observe",
        &format!(
            "EVENT {} project={} tool={}",
            summary.summary, event.project, event.tool_name
        ),
    );

    if matches!(event.tool_name.as_str(), "Write" | "Edit") {
        let branch = event.cwd.as_deref().and_then(db::detect_git_branch);
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

fn insert_memory_event_once(
    conn: &rusqlite::Connection,
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
) -> Result<()> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM events
             WHERE session_id = ?1
               AND project = ?2
               AND event_type = ?3
               AND summary = ?4
               AND COALESCE(detail, '') = COALESCE(?5, '')
               AND COALESCE(files, '') = COALESCE(?6, '')
               AND COALESCE(exit_code, -2147483648) = COALESCE(?7, -2147483648)
             LIMIT 1",
            rusqlite::params![
                &event.session_id,
                &event.project,
                &summary.event_type,
                &summary.summary,
                summary.detail.as_deref(),
                summary.files_json.as_deref(),
                summary.exit_code
            ],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
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
    }
    Ok(())
}

fn detect_adapter_for_host(
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
fn record_capture_event(
    conn: &rusqlite::Connection,
    host: &str,
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
) -> Result<i64> {
    let content = capture_event_content(event, summary);
    let event_id = db::unique_capture_event_id("tool_result", &content);
    record_capture_event_with_id(conn, host, &event_id, event, summary)
}

fn record_capture_event_with_id(
    conn: &rusqlite::Connection,
    host: &str,
    event_id: &str,
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
) -> Result<i64> {
    let content = capture_event_content(event, summary);
    let outcome = db::record_captured_event_with_id(
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
    )?;
    Ok(outcome.event_row_id)
}

fn capture_event_content(
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
) -> String {
    let git_branch = event.cwd.as_deref().and_then(db::detect_git_branch);
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
        "git_branch": git_branch.as_deref(),
    })
    .to_string()
}

fn event_skip_reason(
    adapter: &dyn crate::adapter::ToolAdapter,
    event: &crate::adapter::ParsedHookEvent,
) -> Option<&'static str> {
    if adapter.should_skip(event) {
        return Some("adapter_skip");
    }
    if event.tool_name != "Bash" {
        return None;
    }

    if adapter.name() == "codex-cli" && !codex_bash_observe_enabled() {
        return Some("codex_bash_disabled");
    }

    if event
        .tool_input
        .as_ref()
        .and_then(|value| value["command"].as_str())
        .is_some_and(|command| adapter.should_skip_bash(command))
    {
        return Some("bash_read_only");
    }

    None
}

fn skip_detail(event: &crate::adapter::ParsedHookEvent) -> Option<&str> {
    event
        .tool_input
        .as_ref()
        .and_then(|value| value["command"].as_str())
        .map(|command| db::truncate_str(command, 240))
}

fn codex_bash_observe_enabled() -> bool {
    std::env::var("REMEM_ENABLE_CODEX_BASH_OBSERVE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crate::adapter::{codex::CodexAdapter, EventSummary, ParsedHookEvent};
    use crate::db::{self, test_support::ScopedTestDataDir};

    use super::super::spill::spill_capture_event;
    use super::{
        event_skip_reason, observe_input, record_capture_event, session_init_event,
        SPILL_REASON_CAPTURE_PERSISTENCE_FAILED,
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn codex_bash_event() -> ParsedHookEvent {
        ParsedHookEvent {
            session_id: "session".to_string(),
            cwd: Some("/tmp".to_string()),
            project: "/tmp".to_string(),
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
            event_skip_reason(&CodexAdapter, &codex_bash_event()),
            Some("codex_bash_disabled")
        );
    }

    #[tokio::test]
    async fn observe_records_codex_bash_skip_in_drop_ledger() -> anyhow::Result<()> {
        let _guard = ENV_LOCK.lock().expect("env lock should acquire");
        unsafe { std::env::remove_var("REMEM_ENABLE_CODEX_BASH_OBSERVE") };
        let _test_dir = ScopedTestDataDir::new("observe-codex-bash-drop");
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

    #[test]
    fn codex_bash_observe_can_be_enabled_explicitly() {
        let _guard = ENV_LOCK.lock().expect("env lock should acquire");
        unsafe { std::env::set_var("REMEM_ENABLE_CODEX_BASH_OBSERVE", "1") };
        let event = codex_bash_event();
        let skipped = event_skip_reason(&CodexAdapter, &event);
        unsafe { std::env::remove_var("REMEM_ENABLE_CODEX_BASH_OBSERVE") };

        assert_eq!(skipped, None);
    }

    #[test]
    fn session_init_skips_empty_hook_input() {
        let _test_dir = ScopedTestDataDir::new("session-init-empty");

        assert!(session_init_event("", Some("claude-code")).is_none());
    }

    #[test]
    fn session_init_accepts_claude_user_prompt_submit_shape() {
        let _test_dir = ScopedTestDataDir::new("session-init-user-prompt");
        let input = serde_json::json!({
            "session_id": "sess-user-prompt",
            "cwd": "/tmp/remem",
            "hook_event_name": "UserPromptSubmit",
            "prompt": "hello"
        })
        .to_string();

        let Some(event) = session_init_event(&input, Some("claude-code")) else {
            panic!("event should parse");
        };

        assert_eq!(event.session_id, "sess-user-prompt");
        assert_eq!(event.project, "/tmp/remem");
    }

    #[test]
    fn record_capture_event_writes_ledger_and_coalesced_task() {
        let _test_dir = ScopedTestDataDir::new("observe-capture-ledger");
        let conn = db::open_db().expect("db should open");
        let event = ParsedHookEvent {
            session_id: "sess-observe".to_string(),
            cwd: Some("/tmp/remem".to_string()),
            project: "/tmp/remem".to_string(),
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

        record_capture_event(&conn, "claude-code", &event, &summary)
            .expect("capture event should write");

        let captured_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))
            .expect("captured count should query");
        let task_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM extraction_tasks", [], |row| {
                row.get(0)
            })
            .expect("task count should query");
        assert_eq!(captured_count, 1);
        assert_eq!(task_count, 1);
    }

    #[tokio::test]
    async fn observe_writes_only_normalized_capture_pipeline() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("observe-normalized-only");
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
        let replayed_drop_reason: String = conn.query_row(
            "SELECT reason FROM capture_drop_events WHERE session_id = 'sess-persist-fail'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(
            replayed_drop_reason,
            SPILL_REASON_CAPTURE_PERSISTENCE_FAILED
        );
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
