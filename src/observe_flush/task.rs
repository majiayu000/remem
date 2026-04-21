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

    let existing_context = build_existing_context(conn, project)
        .map_err(|err| {
            crate::log::warn(
                "flush",
                &format!("existing context failed (continuing): {}", err),
            );
        })
        .unwrap_or_default();
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

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use crate::db::PendingObservation;

    use super::flush_single_task;

    /// Regression guard for https://github.com/majiayu000/remem/issues/30.
    ///
    /// When the DB is missing the context tables, `build_existing_context` returns
    /// `Err`.  The fix at task.rs:32 must swallow that error as a warning and
    /// continue — not propagate it.  We verify by checking that the error returned
    /// (which will be from the downstream AI call, not from SQLite) does NOT
    /// contain the SQLite "no such table" sentinel that would indicate the context
    /// error leaked through.
    #[tokio::test]
    async fn flush_single_task_continues_when_context_tables_missing() -> anyhow::Result<()> {
        // Empty in-memory DB — no tables at all, so build_existing_context fails.
        let mut conn = Connection::open_in_memory()?;

        let long_response = "A".repeat(200); // > MIN_TASK_RESPONSE_LEN (100)
        let pending = PendingObservation {
            id: 1,
            session_id: "sess-test".to_string(),
            project: "test-proj".to_string(),
            tool_name: "Bash".to_string(),
            tool_input: Some("echo hello".to_string()),
            tool_response: Some(long_response),
            cwd: None,
            created_at_epoch: 0,
            updated_at_epoch: 0,
            status: "claimed".to_string(),
            attempt_count: 1,
            next_retry_epoch: None,
            last_error: None,
        };

        let result = flush_single_task(&mut conn, "sess-test", "test-proj", "owner", &pending).await;

        // build_existing_context queries `observations` and `memories`.  If its
        // error leaks through, the returned Err will contain one of those table
        // names.  Any other error (AI call failure, missing pending_observations
        // table, etc.) is irrelevant to this regression guard.
        if let Err(ref e) = result {
            let msg = e.to_string();
            assert!(
                !msg.contains("no such table: observations")
                    && !msg.contains("no such table: memories"),
                "build_existing_context error leaked into flush_single_task: {}",
                e
            );
        }
        // Ok(_) is also acceptable (e.g. if the full pipeline completes in CI).
        Ok(())
    }
}
