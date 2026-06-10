use anyhow::Result;

use crate::db;

use super::native::sync_native_memory;

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
        return Ok(());
    };
    if adapter.should_skip(&event) || should_skip_bash_event(adapter, &event) {
        return Ok(());
    }

    let Some(summary) = adapter.classify_event(&event) else {
        return Ok(());
    };

    let conn = db::open_db()?;
    let capture_host = host
        .map(crate::runtime_config::normalize_host)
        .unwrap_or_else(|| adapter.name().to_string());
    record_capture_event(&conn, &capture_host, &event, &summary)?;
    crate::memory::insert_event(
        &conn,
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

fn record_capture_event(
    conn: &rusqlite::Connection,
    host: &str,
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
) -> Result<()> {
    let git_branch = event.cwd.as_deref().and_then(db::detect_git_branch);
    let content = serde_json::json!({
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
    .to_string();
    db::record_captured_event(
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
    )?;
    Ok(())
}

fn should_skip_bash_event(
    adapter: &dyn crate::adapter::ToolAdapter,
    event: &crate::adapter::ParsedHookEvent,
) -> bool {
    if event.tool_name != "Bash" {
        return false;
    }

    if adapter.name() == "codex-cli" && !codex_bash_observe_enabled() {
        return true;
    }

    event
        .tool_input
        .as_ref()
        .and_then(|value| value["command"].as_str())
        .is_some_and(|command| adapter.should_skip_bash(command))
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
}
