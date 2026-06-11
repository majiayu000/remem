use anyhow::{Context, Result};

use crate::db;

use super::native::sync_native_memory;
use super::spill::{replay_capture_spills, write_capture_spill};

pub async fn session_init(host: Option<&str>) -> Result<()> {
    let timer = crate::log::Timer::start("session-init", "");
    let input = std::io::read_to_string(std::io::stdin())?;
    crate::log::debug(
        "session-init",
        &format!("raw input: {}", crate::db::truncate_str(&input, 500)),
    );

    let Some(event) = session_init_event(&input, host) else {
        record_capture_audit_best_effort(
            host.map(crate::runtime_config::normalize_host),
            None,
            None,
            "adapter_mismatch",
            Some("session-init hook input did not match a known adapter"),
            None,
        );
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
        record_capture_audit_best_effort(
            host.map(crate::runtime_config::normalize_host),
            None,
            None,
            "adapter_mismatch",
            Some("observe hook input did not match a known adapter"),
            None,
        );
        return Ok(());
    };
    if let Some((reason, detail)) = skip_reason(adapter, &event) {
        let payload = capture_audit_payload(&event);
        record_capture_audit_best_effort(
            Some(capture_host(host, adapter)),
            Some(adapter.name()),
            Some(&event),
            reason,
            Some(&detail),
            Some(&payload),
        );
        return Ok(());
    }

    let Some(summary) = adapter.classify_event(&event) else {
        let payload = capture_audit_payload(&event);
        record_capture_audit_best_effort(
            Some(capture_host(host, adapter)),
            Some(adapter.name()),
            Some(&event),
            "unclassified",
            Some("adapter accepted event but produced no summary"),
            Some(&payload),
        );
        return Ok(());
    };

    let capture_host = capture_host(host, adapter);
    let content = build_capture_content(&event, &summary);
    let event_id = db::unique_capture_event_id("tool_result", &content);
    let conn = match db::open_db() {
        Ok(conn) => conn,
        Err(error) => {
            let spill = write_capture_spill(
                &event_id,
                &capture_host,
                adapter.name(),
                &event,
                &summary,
                &content,
                &error.to_string(),
            )
            .with_context(|| {
                format!(
                    "database open failed and capture spill write failed for session {}",
                    event.session_id
                )
            })?;
            crate::log::warn(
                "observe",
                &format!(
                    "database open failed; spilled capture event {} to {}",
                    spill.event_id,
                    spill.path.display()
                ),
            );
            return Err(error).context("database open failed after capture event was spilled");
        }
    };
    if let Err(error) = replay_capture_spills(&conn) {
        crate::log::warn(
            "observe",
            &format!("capture spill replay failed: {}", error),
        );
    }
    if let Err(error) =
        persist_capture_event(&conn, &capture_host, &event, &summary, &content, &event_id)
    {
        let spill = write_capture_spill(
            &event_id,
            &capture_host,
            adapter.name(),
            &event,
            &summary,
            &content,
            &error.to_string(),
        )
        .with_context(|| {
            format!(
                "capture persistence failed and spill write failed for session {}",
                event.session_id
            )
        })?;
        crate::log::warn(
            "observe",
            &format!(
                "capture persistence failed; spilled event {} to {}",
                spill.event_id,
                spill.path.display()
            ),
        );
        return Err(error).context("capture persistence failed after event was spilled");
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
                sync_native_memory(&conn, &event.session_id, file_path, branch.as_deref())
            {
                crate::log::warn("observe", &format!("native memory sync failed: {}", error));
                if let Err(audit_error) = record_capture_audit_for_event(
                    &conn,
                    Some(&capture_host),
                    Some(adapter.name()),
                    Some(&event),
                    "native_read_error",
                    Some(&error.to_string()),
                    Some(&capture_audit_payload(&event)),
                ) {
                    crate::log::warn(
                        "observe",
                        &format!("native memory audit failed: {}", audit_error),
                    );
                }
            }
        }
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

fn capture_host(host: Option<&str>, adapter: &dyn crate::adapter::ToolAdapter) -> String {
    host.map(crate::runtime_config::normalize_host)
        .unwrap_or_else(|| adapter.name().to_string())
}

#[cfg(test)]
fn record_capture_event(
    conn: &rusqlite::Connection,
    host: &str,
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
) -> Result<()> {
    let content = build_capture_content(event, summary);
    record_capture_event_content(conn, host, event, summary, &content)
}

#[cfg(test)]
fn record_capture_event_content(
    conn: &rusqlite::Connection,
    host: &str,
    event: &crate::adapter::ParsedHookEvent,
    _summary: &crate::adapter::EventSummary,
    content: &str,
) -> Result<()> {
    record_capture_event_content_with_id(conn, host, event, content, None)
}

fn record_capture_event_content_with_id(
    conn: &rusqlite::Connection,
    host: &str,
    event: &crate::adapter::ParsedHookEvent,
    content: &str,
    event_id_override: Option<&str>,
) -> Result<()> {
    let input = db::CaptureEventInput {
        host,
        session_id: &event.session_id,
        project: &event.project,
        cwd: event.cwd.as_deref(),
        event_type: "tool_result",
        role: None,
        tool_name: Some(&event.tool_name),
        content,
        task_kind: Some(db::ExtractionTaskKind::ObservationExtract),
    };
    match event_id_override {
        Some(event_id) => {
            db::record_captured_event_with_id(conn, &input, Some(event_id))?;
        }
        None => {
            db::record_captured_event(conn, &input)?;
        }
    }
    Ok(())
}

fn persist_capture_event(
    conn: &rusqlite::Connection,
    host: &str,
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
    content: &str,
    event_id: &str,
) -> Result<()> {
    record_capture_event_content_with_id(conn, host, event, content, Some(event_id))?;
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
    Ok(())
}

fn build_capture_content(
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

fn capture_audit_payload(event: &crate::adapter::ParsedHookEvent) -> String {
    serde_json::json!({
        "session_id": &event.session_id,
        "project": &event.project,
        "cwd": event.cwd.as_deref(),
        "tool_name": &event.tool_name,
        "tool_input": event.tool_input.as_ref(),
        "tool_response": event.tool_response.as_ref(),
    })
    .to_string()
}

fn record_capture_audit_best_effort(
    host: Option<String>,
    adapter: Option<&str>,
    event: Option<&crate::adapter::ParsedHookEvent>,
    reason: &str,
    detail: Option<&str>,
    payload: Option<&str>,
) {
    let conn = match db::open_db() {
        Ok(conn) => conn,
        Err(error) => {
            crate::log::warn(
                "observe",
                &format!("capture audit unavailable for {reason}: {}", error),
            );
            return;
        }
    };
    if let Err(error) = replay_capture_spills(&conn) {
        crate::log::warn(
            "observe",
            &format!("capture spill replay failed: {}", error),
        );
    }
    if let Err(error) = record_capture_audit_for_event(
        &conn,
        host.as_deref(),
        adapter,
        event,
        reason,
        detail,
        payload,
    ) {
        crate::log::warn(
            "observe",
            &format!("capture audit write failed for {reason}: {}", error),
        );
    }
}

fn record_capture_audit_for_event(
    conn: &rusqlite::Connection,
    host: Option<&str>,
    adapter: Option<&str>,
    event: Option<&crate::adapter::ParsedHookEvent>,
    reason: &str,
    detail: Option<&str>,
    payload: Option<&str>,
) -> Result<()> {
    db::record_capture_audit_event(
        conn,
        &db::CaptureAuditInput {
            host,
            adapter,
            session_id: event.map(|event| event.session_id.as_str()),
            project: event.map(|event| event.project.as_str()),
            cwd: event.and_then(|event| event.cwd.as_deref()),
            tool_name: event.map(|event| event.tool_name.as_str()),
            reason,
            detail,
            payload,
        },
    )?;
    Ok(())
}

#[cfg(test)]
fn should_skip_bash_event(
    adapter: &dyn crate::adapter::ToolAdapter,
    event: &crate::adapter::ParsedHookEvent,
) -> bool {
    bash_skip_detail(adapter, event).is_some()
}

fn skip_reason(
    adapter: &dyn crate::adapter::ToolAdapter,
    event: &crate::adapter::ParsedHookEvent,
) -> Option<(&'static str, String)> {
    if adapter.should_skip(event) {
        return Some((
            "tool_skipped",
            format!("adapter skipped tool {}", event.tool_name),
        ));
    }
    bash_skip_detail(adapter, event).map(|detail| ("bash_skipped", detail))
}

fn bash_skip_detail(
    adapter: &dyn crate::adapter::ToolAdapter,
    event: &crate::adapter::ParsedHookEvent,
) -> Option<String> {
    if event.tool_name != "Bash" {
        return None;
    }

    if adapter.name() == "codex-cli" && !codex_bash_observe_enabled() {
        return Some("codex bash observe disabled".to_string());
    }

    event
        .tool_input
        .as_ref()
        .and_then(|value| value["command"].as_str())
        .filter(|command| adapter.should_skip_bash(command))
        .map(|command| {
            format!(
                "adapter skipped bash command: {}",
                db::truncate_str(command, 120)
            )
        })
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

    use super::{observe_input, record_capture_event, session_init_event, should_skip_bash_event};

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

    fn audit_count(conn: &rusqlite::Connection, reason: &str) -> anyhow::Result<i64> {
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM capture_audit_events WHERE reason = ?1",
            [reason],
            |row| row.get(0),
        )?)
    }

    #[test]
    fn codex_bash_observe_skips_by_default() {
        let _guard = ENV_LOCK.lock().expect("env lock should acquire");
        unsafe { std::env::remove_var("REMEM_ENABLE_CODEX_BASH_OBSERVE") };

        assert!(should_skip_bash_event(&CodexAdapter, &codex_bash_event()));
    }

    #[test]
    fn codex_bash_observe_can_be_enabled_explicitly() {
        let _guard = ENV_LOCK.lock().expect("env lock should acquire");
        unsafe { std::env::set_var("REMEM_ENABLE_CODEX_BASH_OBSERVE", "1") };
        let event = codex_bash_event();
        let skipped = should_skip_bash_event(&CodexAdapter, &event);
        unsafe { std::env::remove_var("REMEM_ENABLE_CODEX_BASH_OBSERVE") };

        assert!(!skipped);
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
    async fn observe_spills_write_failure_and_replays_without_duplicate_capture(
    ) -> anyhow::Result<()> {
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
            .expect_err("failed event insert should spill and return an error");
        assert!(
            err.to_string().contains("capture persistence failed"),
            "{err}"
        );
        assert_eq!(crate::observe::capture_spill_stats()?.pending_files, 1);

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
        assert_eq!(crate::observe::capture_spill_stats()?.pending_files, 0);
        Ok(())
    }

    #[tokio::test]
    async fn observe_audits_no_adapter_match() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("observe-audit-no-adapter");

        observe_input("{}", Some("claude-code")).await?;

        let conn = db::open_db()?;
        assert_eq!(audit_count(&conn, "adapter_mismatch")?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn observe_audits_tool_skip() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("observe-audit-tool-skip");
        let input = serde_json::json!({
            "session_id": "sess-skip",
            "cwd": "/tmp/remem",
            "tool_name": "Read",
            "tool_input": {"file_path": "src/lib.rs"},
            "tool_response": {"content": "source"}
        })
        .to_string();

        observe_input(&input, Some("claude-code")).await?;

        let conn = db::open_db()?;
        assert_eq!(audit_count(&conn, "tool_skipped")?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn observe_audits_codex_bash_disabled() -> anyhow::Result<()> {
        let _guard = ENV_LOCK.lock().expect("env lock should acquire");
        unsafe { std::env::remove_var("REMEM_ENABLE_CODEX_BASH_OBSERVE") };
        let _test_dir = ScopedTestDataDir::new("observe-audit-codex-bash");
        let input = serde_json::json!({
            "session_id": "sess-bash-skip",
            "cwd": "/tmp/remem",
            "tool_name": "Bash",
            "tool_input": {"command": "cargo test"},
            "tool_response": {"exitCode": 0}
        })
        .to_string();

        observe_input(&input, Some("codex-cli")).await?;

        let conn = db::open_db()?;
        assert_eq!(audit_count(&conn, "bash_skipped")?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn observe_audits_unclassified_event() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("observe-audit-unclassified");
        let input = serde_json::json!({
            "session_id": "sess-unclassified",
            "cwd": "/tmp/remem",
            "tool_name": "Edit",
            "tool_input": {"path": "src/lib.rs"},
            "tool_response": {"content": "edited"}
        })
        .to_string();

        observe_input(&input, Some("claude-code")).await?;

        let conn = db::open_db()?;
        assert_eq!(audit_count(&conn, "unclassified")?, 1);
        Ok(())
    }
}
