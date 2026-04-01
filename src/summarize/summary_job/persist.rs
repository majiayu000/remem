use anyhow::Result;

use crate::db;
use crate::memory_format;

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
    let _deleted = db::finalize_summarize(
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
    )?;
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
    }
    Ok(())
}

pub(super) fn sync_native_memory(cwd: &str, project: &str) {
    if let Err(err) = crate::claude_memory::sync_to_claude_memory(cwd, project) {
        crate::log::warn(
            "summary-job",
            &format!("claude memory sync failed: {}", err),
        );
    }
}

fn push_summary_tag(parts: &mut Vec<String>, tag: &str, value: Option<&str>) {
    if let Some(value) = value {
        parts.push(format!(
            "<{tag}>{}</{tag}>",
            memory_format::xml_escape_text(value)
        ));
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
