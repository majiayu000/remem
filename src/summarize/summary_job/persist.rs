use anyhow::{Context, Result};

use crate::db;
use crate::memory::format;

use super::super::parse::ParsedSummary;

pub(super) fn build_existing_summary_context(
    conn: &rusqlite::Connection,
    memory_sid: &str,
    project: &str,
) -> Result<String> {
    let Some(prev) = db::get_summary_by_session(conn, memory_sid, project)? else {
        return Ok(String::new());
    };

    let mut parts = Vec::new();
    push_summary_tag(&mut parts, "request", prev.request.as_deref());
    push_summary_tag(&mut parts, "completed", prev.completed.as_deref());
    push_summary_tag(&mut parts, "decisions", prev.decisions.as_deref());
    push_summary_tag(&mut parts, "learned", prev.learned.as_deref());
    push_summary_tag(&mut parts, "next_steps", prev.next_steps.as_deref());
    push_summary_tag(&mut parts, "preferences", prev.preferences.as_deref());

    Ok(format!(
        "<existing_summary>\n{}\n</existing_summary>\n\n",
        parts.join("\n")
    ))
}

pub(super) fn finalize_summary(
    conn: &mut rusqlite::Connection,
    session_id: &str,
    memory_sid: &str,
    project: &str,
    msg_hash: &str,
    summary: ParsedSummary,
) -> Result<()> {
    let usage = summary_text_usage(&summary);
    let _deleted = match db::finalize_summarize(
        conn,
        memory_sid,
        project,
        msg_hash,
        summary.request.as_deref(),
        summary.completed.as_deref(),
        summary.decisions.as_deref(),
        summary.learned.as_deref(),
        summary.next_steps.as_deref(),
        summary.preferences.as_deref(),
        None,
        usage,
    ) {
        Ok(deleted) => deleted,
        Err(err) => {
            release_lock_after_error(conn, project, "finalize-failure");
            return Err(err);
        }
    };
    db::release_summarize_lock(conn, project)?;
    crate::log::info(
        "summary-job",
        &format!("saved summary project={} session={}", project, session_id),
    );

    if let Err(err) = crate::memory::promote_summary_to_memories(
        conn,
        session_id,
        project,
        summary.request.as_deref(),
        summary.decisions.as_deref(),
        summary.learned.as_deref(),
        summary.preferences.as_deref(),
    ) {
        crate::log::warn("summary-job", &format!("memory promotion failed: {}", err));
        db::clear_summarize_cooldown_for_message(conn, project, msg_hash)
            .context("failed to clear summary retry marker after memory promotion failure")?;
        return Err(err).context("memory promotion failed");
    }
    Ok(())
}

pub(super) fn sync_native_memory(cwd: &str, project: &str) {
    if let Err(err) = crate::context::claude_memory::sync_to_claude_memory(cwd, project) {
        crate::log::warn(
            "summary-job",
            &format!("claude memory sync failed: {}", err),
        );
    }
}

fn push_summary_tag(parts: &mut Vec<String>, tag: &str, value: Option<&str>) {
    if let Some(value) = value {
        parts.push(format!("<{tag}>{}</{tag}>", format::xml_escape_text(value)));
    }
}

fn summary_text_usage(summary: &ParsedSummary) -> i64 {
    let total_len = [
        summary.request.as_deref(),
        summary.completed.as_deref(),
        summary.decisions.as_deref(),
        summary.learned.as_deref(),
        summary.next_steps.as_deref(),
        summary.preferences.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::len)
    .sum::<usize>();
    (total_len / 4) as i64
}

fn release_lock_after_error(conn: &rusqlite::Connection, project: &str, reason: &str) {
    if let Err(e) = db::release_summarize_lock(conn, project) {
        crate::log::error(
            "summary-job",
            &format!(
                "[LOCK LEAK] failed to release summarize lock for {project} after {reason}: {e}"
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rusqlite::{params, Connection};

    use crate::{db, summarize::ParsedSummary};

    use super::finalize_summary;

    #[test]
    fn promotion_failure_releases_lock_and_clears_retry_marker() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;

        let project = "proj/promotion-failure";
        let session_id = "content-session-1";
        let memory_sid = "memory-session-1";
        let msg_hash = "message-hash-1";

        assert!(db::try_acquire_summarize_lock(&mut conn, project, 60)?);
        conn.execute_batch("DROP TABLE memories;")?;

        let err = finalize_summary(
            &mut conn,
            session_id,
            memory_sid,
            project,
            msg_hash,
            ParsedSummary {
                request: Some("Capture decisions from a summary".to_string()),
                completed: Some("Saved session summary".to_string()),
                decisions: Some(
                    "Use a retryable worker failure when summary promotion cannot persist"
                        .to_string(),
                ),
                learned: None,
                next_steps: None,
                preferences: None,
            },
        )
        .expect_err("promotion failure should surface to the worker");

        assert!(
            err.to_string().contains("memory promotion failed"),
            "unexpected error: {err:#}"
        );

        let locks: i64 = conn.query_row(
            "SELECT COUNT(*) FROM summarize_locks WHERE project = ?1",
            params![project],
            |row| row.get(0),
        )?;
        assert_eq!(locks, 0, "summarize lock should be released");

        assert!(
            !db::is_summarize_on_cooldown(&conn, project, 60 * 60)?,
            "cooldown should not suppress retry after promotion failure"
        );
        assert!(
            !db::is_duplicate_message(&conn, project, msg_hash)?,
            "duplicate marker should not suppress retry after promotion failure"
        );

        Ok(())
    }
}
