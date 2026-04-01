use anyhow::Result;

use crate::db;
use crate::memory_format;

use super::constants::{MIN_TASK_RESPONSE_LEN, TASK_OBSERVATION_PROMPT};
use super::context::{build_existing_context, build_session_events_xml};
use super::persist::persist_flush_batch;

pub(crate) async fn flush_single_task(
    conn: &mut rusqlite::Connection,
    session_id: &str,
    project: &str,
    lease_owner: &str,
    pending: &db::PendingObservation,
) -> Result<usize> {
    let response_text = pending.tool_response.as_deref().unwrap_or("");
    if response_text.len() < MIN_TASK_RESPONSE_LEN {
        let reason = format!(
            "task response too short: {}B < {}B",
            response_text.len(),
            MIN_TASK_RESPONSE_LEN
        );
        crate::log::warn(
            "flush-task",
            &format!("mark failed Task id={} ({})", pending.id, reason),
        );
        db::fail_pending_claimed(conn, lease_owner, &[pending.id], &reason)?;
        return Ok(0);
    }

    let existing_context = build_existing_context(conn, project).unwrap_or_default();
    let events = build_session_events_xml(std::slice::from_ref(pending));
    let user_message = format!(
        "{}<session_events>\n{}</session_events>",
        existing_context, events
    );

    let ai_start = std::time::Instant::now();
    let response = crate::ai::call_ai(
        TASK_OBSERVATION_PROMPT,
        &user_message,
        crate::ai::UsageContext {
            project: Some(project),
            operation: "flush-task",
        },
    )
    .await?;
    let ai_ms = ai_start.elapsed().as_millis();

    crate::log::info(
        "flush-task",
        &format!("AI response {}ms {}B", ai_ms, response.len()),
    );

    let observations = memory_format::parse_observations(&response);
    if observations.is_empty() {
        let reason = "no observations extracted from task response";
        crate::log::warn("flush-task", reason);
        db::fail_pending_claimed(conn, lease_owner, &[pending.id], reason)?;
        return Ok(0);
    }

    let usage = response.len() as i64 / 4;
    let branch = pending.cwd.as_deref().and_then(db::detect_git_branch);
    let commit_sha = pending.cwd.as_deref().and_then(db::detect_git_commit);
    persist_flush_batch(
        conn,
        session_id,
        project,
        lease_owner,
        std::slice::from_ref(pending),
        &observations,
        usage,
        branch.as_deref(),
        commit_sha.as_deref(),
    )?;

    Ok(observations.len())
}
