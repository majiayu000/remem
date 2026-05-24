use anyhow::Result;

use crate::db;

use super::native::sync_native_memory;

pub async fn session_init() -> Result<()> {
    let timer = crate::log::Timer::start("session-init", "");
    let input = std::io::read_to_string(std::io::stdin())?;
    crate::log::debug(
        "session-init",
        &format!("raw input: {}", crate::db::truncate_str(&input, 500)),
    );

    let Some((_adapter, event)) = crate::adapter::detect_adapter(&input) else {
        anyhow::bail!("no adapter matched session_init input");
    };

    crate::log::info(
        "session-init",
        &format!("project={} session={}", event.project, event.session_id),
    );

    let conn = db::open_db()?;
    db::upsert_session(&conn, &event.session_id, &event.project, None)?;

    timer.done(&format!("project={}", event.project));
    Ok(())
}

pub async fn observe() -> Result<()> {
    let input = std::io::read_to_string(std::io::stdin())?;

    let Some((adapter, event)) = crate::adapter::detect_adapter(&input) else {
        return Ok(());
    };
    if adapter.should_skip(&event) || should_skip_bash_event(adapter, &event) {
        return Ok(());
    }

    let Some(summary) = adapter.classify_event(&event) else {
        return Ok(());
    };

    let conn = db::open_db()?;
    record_capture_event(&conn, adapter.name(), &event, &summary)?;
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

    let tool_input_str =
        crate::adapter::common::pending_tool_input(&event.tool_name, &event.tool_input);
    let tool_response_str = crate::adapter::common::pending_tool_response(
        &event.tool_name,
        &event.tool_input,
        &event.tool_response,
    );
    db::enqueue_pending(
        &conn,
        adapter.name(),
        &event.session_id,
        &event.project,
        &event.tool_name,
        tool_input_str.as_deref(),
        tool_response_str.as_deref(),
        event.cwd.as_deref(),
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
        "tool_input": event.tool_input.as_ref(),
        "tool_response": event.tool_response.as_ref(),
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

    use super::{record_capture_event, should_skip_bash_event};

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
}
