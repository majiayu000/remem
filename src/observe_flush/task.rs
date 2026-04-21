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

    use crate::db::test_support::ScopedTestDataDir;
    use crate::db::PendingObservation;

    use super::flush_single_task;

    fn make_pending(long_response: String) -> PendingObservation {
        PendingObservation {
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
        }
    }

    /// Regression guard for https://github.com/majiayu000/remem/issues/30.
    ///
    /// When `build_existing_context` fails (e.g. "no such table: observations"),
    /// the fix at task.rs:32 must emit a WARN log entry and then continue —
    /// not propagate the error.  This test verifies the warning was actually
    /// written to the log file.  A bare `.unwrap_or_default()` (the pre-fix
    /// regression) produces no log output and would therefore fail this assertion,
    /// distinguishing it from the fixed `.map_err(warn).unwrap_or_default()` path.
    // ScopedTestDataDir holds a std::sync::MutexGuard (!Send) to serialize env-var
    // access. Holding it across .await is intentional here: the warning fires
    // before the first await, and the guard must stay alive until we read the log
    // file after the call returns. #[tokio::test] uses a current-thread runtime
    // so the future need not be Send.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn flush_single_task_warns_on_context_build_failure() -> anyhow::Result<()> {
        // Redirect REMEM_DATA_DIR so we can read the log file and assert the
        // warning was emitted.
        let data_dir = ScopedTestDataDir::new("flush-single-warn");
        let log_path = data_dir.path.join("remem.log");

        // Empty in-memory DB — no tables at all, so build_existing_context fails.
        let mut conn = Connection::open_in_memory()?;
        let pending = make_pending("A".repeat(200)); // > MIN_TASK_RESPONSE_LEN (100)

        let result = flush_single_task(&mut conn, "sess-test", "test-proj", "owner", &pending).await;

        // The SQLite context error must NOT propagate as the returned Err.
        if let Err(ref e) = result {
            let msg = e.to_string();
            assert!(
                !msg.contains("no such table: observations")
                    && !msg.contains("no such table: memories"),
                "build_existing_context error leaked into flush_single_task: {}",
                e
            );
        }

        // Core regression guard: the WARN line must be present in the log file.
        // The pre-fix code (bare .unwrap_or_default()) never wrote this line,
        // so this assertion fails on the old implementation and passes on the fix.
        let log_content = std::fs::read_to_string(&log_path).unwrap_or_default();
        assert!(
            log_content.contains("existing context failed (continuing)"),
            "expected WARN 'existing context failed (continuing)' in {log_path:?}, got:\n{log_content}"
        );

        Ok(())
    }
}
