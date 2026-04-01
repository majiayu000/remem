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

    let branch = event.cwd.as_deref().and_then(db::detect_git_branch);
    let _commit_sha = event.cwd.as_deref().and_then(db::detect_git_commit);

    let conn = db::open_db()?;
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

    let tool_input_str = event.tool_input.as_ref().map(|value| value.to_string());
    let tool_response_str = event.tool_response.as_ref().map(|value| value.to_string());
    db::enqueue_pending(
        &conn,
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
            "EVENT {} project={} branch={:?}",
            summary.summary, event.project, branch
        ),
    );

    if matches!(event.tool_name.as_str(), "Write" | "Edit") {
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

fn should_skip_bash_event(
    adapter: &dyn crate::adapter::ToolAdapter,
    event: &crate::adapter::ParsedHookEvent,
) -> bool {
    if event.tool_name != "Bash" {
        return false;
    }

    event
        .tool_input
        .as_ref()
        .and_then(|value| value["command"].as_str())
        .is_some_and(|command| adapter.should_skip_bash(command))
}
