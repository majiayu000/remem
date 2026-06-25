use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};

use crate::db;
use crate::memory::format;
use crate::workstream::ParsedWorkStream;

use super::super::parse::ParsedSummary;

pub(super) fn build_existing_summary_context(
    conn: &rusqlite::Connection,
    memory_sid: &str,
    project: &str,
) -> Result<String> {
    let prev = db::get_summary_by_session(conn, memory_sid, project)?;
    let prev_workstream = get_linked_workstream_context(conn, memory_sid, project)?;

    if prev.is_none() && prev_workstream.is_none() {
        return Ok(String::new());
    }

    let mut parts = Vec::new();
    if let Some(prev) = prev {
        push_summary_tag(&mut parts, "request", prev.request.as_deref());
        push_summary_tag(&mut parts, "completed", prev.completed.as_deref());
        push_summary_tag(&mut parts, "decisions", prev.decisions.as_deref());
        push_summary_tag(&mut parts, "learned", prev.learned.as_deref());
        push_summary_tag(&mut parts, "next_steps", prev.next_steps.as_deref());
        push_summary_tag(&mut parts, "preferences", prev.preferences.as_deref());
    }
    if let Some(prev_workstream) = prev_workstream {
        push_summary_tag(&mut parts, "workstream", Some(&prev_workstream.title));
        push_summary_tag(
            &mut parts,
            "workstream_progress",
            prev_workstream.progress.as_deref(),
        );
        push_summary_tag(
            &mut parts,
            "workstream_next",
            prev_workstream.next_action.as_deref(),
        );
        push_summary_tag(
            &mut parts,
            "workstream_blockers",
            prev_workstream.blockers.as_deref(),
        );
    }

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

    let linked_commits =
        crate::git_trace::link_observed_commits_for_session(conn, project, session_id, memory_sid)
            .context("failed to link observed commits for summarized session")?;
    if linked_commits > 0 {
        crate::log::info(
            "summary-job",
            &format!(
                "linked {} observed commit(s) project={} session={}",
                linked_commits, project, session_id
            ),
        );
    }

    if let Some(workstream) = parsed_workstream_from_summary(&summary) {
        match crate::workstream::upsert_workstream_with_match(
            conn,
            project,
            memory_sid,
            &workstream,
        ) {
            Ok(result) => crate::log::info(
                "summary-job",
                &format!(
                    "upserted workstream id={} reason={} project={} session={}",
                    result.id, result.match_reason, project, session_id
                ),
            ),
            Err(err) => crate::log::warn(
                "summary-job",
                &format!("workstream persistence failed: {}", err),
            ),
        }
    }

    if let Err(err) = crate::memory::promote_summary_to_memory_candidates(
        conn,
        session_id,
        project,
        summary.request.as_deref(),
        summary.decisions.as_deref(),
        summary.learned.as_deref(),
        summary.preferences.as_deref(),
    ) {
        crate::log::warn(
            "summary-job",
            &format!("memory candidate promotion failed: {}", err),
        );
        db::clear_summarize_cooldown_for_message(conn, project, msg_hash)
            .context("failed to clear summary retry marker after memory candidate failure")?;
        return Err(err).context("memory candidate promotion failed");
    }
    Ok(())
}

pub(super) fn sync_native_memory(conn: &rusqlite::Connection, cwd: &str, project: &str) {
    if let Err(err) = crate::context::claude_memory::sync_to_claude_memory(conn, cwd, project) {
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
        summary.workstream.as_deref(),
        summary.workstream_progress.as_deref(),
        summary.workstream_next.as_deref(),
        summary.workstream_blockers.as_deref(),
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

#[derive(Debug)]
struct ExistingWorkStreamContext {
    title: String,
    progress: Option<String>,
    next_action: Option<String>,
    blockers: Option<String>,
}

fn get_linked_workstream_context(
    conn: &rusqlite::Connection,
    memory_sid: &str,
    project: &str,
) -> Result<Option<ExistingWorkStreamContext>> {
    conn.query_row(
        "SELECT canonical.title, canonical.progress, canonical.next_action, canonical.blockers
         FROM workstream_sessions wss
         JOIN workstreams linked ON linked.id = wss.workstream_id
         JOIN workstreams canonical ON canonical.id = COALESCE(linked.merged_into_workstream_id, linked.id)
         WHERE wss.memory_session_id = ?1
           AND canonical.merged_into_workstream_id IS NULL
           AND ((canonical.owner_scope = 'repo' AND canonical.owner_key = ?2)
                OR (canonical.owner_scope = 'repo' AND canonical.target_project = ?2)
                OR (canonical.owner_scope = 'workstream' AND canonical.target_project = ?2)
                OR (canonical.owner_scope IS NULL AND canonical.project = ?2))
         ORDER BY wss.linked_at_epoch DESC, canonical.updated_at_epoch DESC
         LIMIT 1",
        params![memory_sid, project],
        |row| {
            Ok(ExistingWorkStreamContext {
                title: row.get(0)?,
                progress: row.get(1)?,
                next_action: row.get(2)?,
                blockers: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn parsed_workstream_from_summary(summary: &ParsedSummary) -> Option<ParsedWorkStream> {
    Some(ParsedWorkStream {
        title: clean_field(summary.workstream.as_deref()),
        progress: clean_field(summary.workstream_progress.as_deref()),
        next_action: clean_field(summary.workstream_next.as_deref()),
        blockers: clean_field(summary.workstream_blockers.as_deref()),
        is_completed: false,
    })
    .filter(|workstream| workstream.title.is_some())
}

fn clean_field(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rusqlite::{params, Connection};

    use crate::{db, summarize::ParsedSummary};

    use super::{build_existing_summary_context, finalize_summary};

    fn setup_conn() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    fn record_summary_evidence(conn: &Connection, session_id: &str, project: &str) -> Result<i64> {
        let outcome = db::record_captured_event(
            conn,
            &db::CaptureEventInput {
                host: "codex-cli",
                session_id,
                project,
                cwd: Some(project),
                event_type: "session_stop",
                role: None,
                tool_name: None,
                content: "summary source payload",
                task_kind: Some(db::ExtractionTaskKind::SessionRollup),
            },
        )?;
        Ok(outcome.event_row_id)
    }

    #[test]
    fn candidate_failure_releases_lock_and_clears_retry_marker() -> Result<()> {
        let mut conn = setup_conn()?;

        let project = "proj/candidate-failure";
        let session_id = "content-session-1";
        let memory_sid = "memory-session-1";
        let msg_hash = "message-hash-1";

        assert!(db::try_acquire_summarize_lock(&mut conn, project, 60)?);

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
                workstream: None,
                workstream_progress: None,
                workstream_next: None,
                workstream_blockers: None,
            },
        )
        .expect_err("missing candidate evidence should surface to the worker");

        assert!(
            err.to_string()
                .contains("memory candidate promotion failed"),
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

    #[test]
    fn finalize_summary_creates_candidates_without_active_memories() -> Result<()> {
        let mut conn = setup_conn()?;
        let project = "test/proj";
        let session_id = "summary-candidate-session";
        let memory_sid = "mem-summary-candidate";
        let evidence_id = record_summary_evidence(&conn, session_id, project)?;

        finalize_summary(
            &mut conn,
            session_id,
            memory_sid,
            project,
            "hash-summary-candidate",
            ParsedSummary {
                request: Some("Repair summary memory governance".to_string()),
                completed: Some("Saved summary row".to_string()),
                decisions: Some(
                    "Use memory candidates for summary-derived durable decisions".to_string(),
                ),
                learned: Some(
                    "FTS5 trigram tokenizer handles CJK search without word boundaries".to_string(),
                ),
                next_steps: None,
                preferences: Some(
                    "Always review summary-derived preferences before activation".to_string(),
                ),
                workstream: None,
                workstream_progress: None,
                workstream_next: None,
                workstream_blockers: None,
            },
        )?;

        let memory_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
        let candidate_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
                row.get(0)
            })?;
        assert_eq!(memory_count, 0);
        assert_eq!(candidate_count, 3);

        let rows = conn
            .prepare(
                "SELECT memory_type, review_status, evidence_event_ids,
                        source_project, owner_scope, owner_key
                 FROM memory_candidates
                 ORDER BY id ASC",
            )?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let evidence_json = serde_json::to_string(&vec![evidence_id])?;
        assert_eq!(
            rows,
            vec![
                (
                    "decision".to_string(),
                    "pending_review".to_string(),
                    evidence_json.clone(),
                    project.to_string(),
                    "repo".to_string(),
                    project.to_string()
                ),
                (
                    "discovery".to_string(),
                    "pending_review".to_string(),
                    evidence_json.clone(),
                    project.to_string(),
                    "repo".to_string(),
                    project.to_string()
                ),
                (
                    "preference".to_string(),
                    "pending_review".to_string(),
                    evidence_json,
                    project.to_string(),
                    "repo".to_string(),
                    project.to_string()
                )
            ]
        );
        Ok(())
    }

    #[test]
    fn finalize_summary_with_no_durable_candidates_does_not_require_evidence() -> Result<()> {
        let mut conn = setup_conn()?;

        finalize_summary(
            &mut conn,
            "summary-no-candidates",
            "mem-summary-no-candidates",
            "test/proj",
            "hash-summary-no-candidates",
            ParsedSummary {
                request: Some("Tiny update".to_string()),
                completed: Some("Done".to_string()),
                decisions: Some("Short".to_string()),
                learned: Some("Also short".to_string()),
                next_steps: None,
                preferences: None,
                workstream: None,
                workstream_progress: None,
                workstream_next: None,
                workstream_blockers: None,
            },
        )?;

        let memory_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
        let candidate_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
                row.get(0)
            })?;
        assert_eq!(memory_count, 0);
        assert_eq!(candidate_count, 0);
        Ok(())
    }

    #[test]
    fn build_existing_summary_context_includes_linked_workstream_fields() -> Result<()> {
        let conn = setup_conn()?;
        conn.execute(
            "INSERT INTO session_summaries
             (memory_session_id, project, request, completed, decisions, learned,
              next_steps, preferences, created_at, created_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                "mem-ctx",
                "test/proj",
                "Repair summary capture",
                "Saved summary row",
                "Use WorkStream table as source of truth",
                "Incremental summaries need linked task state",
                "Run persistence test",
                "Prefer evidence",
                "2026-05-13T00:00:00Z",
                1_768_000_000_i64,
            ],
        )?;
        conn.execute(
            "INSERT INTO workstreams
             (project, title, status, progress, next_action, blockers,
              created_at_epoch, updated_at_epoch)
             VALUES (?1, ?2, 'active', ?3, ?4, ?5, ?6, ?7)",
            params![
                "test/proj",
                "Summary WorkStream Persistence",
                "Parsed fields wired",
                "Finalize summary test",
                "Need clippy",
                1_768_000_000_i64,
                1_768_000_010_i64,
            ],
        )?;
        let workstream_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO workstream_sessions
             (workstream_id, memory_session_id, linked_at_epoch)
             VALUES (?1, ?2, ?3)",
            params![workstream_id, "mem-ctx", 1_768_000_011_i64],
        )?;

        let context = build_existing_summary_context(&conn, "mem-ctx", "test/proj")?;

        assert!(context.contains("<request>Repair summary capture</request>"));
        assert!(context.contains("<workstream>Summary WorkStream Persistence</workstream>"));
        assert!(context.contains("<workstream_progress>Parsed fields wired</workstream_progress>"));
        assert!(context.contains("<workstream_next>Finalize summary test</workstream_next>"));
        assert!(context.contains("<workstream_blockers>Need clippy</workstream_blockers>"));
        Ok(())
    }

    #[test]
    fn finalize_summary_persists_non_empty_workstream() -> Result<()> {
        let mut conn = setup_conn()?;
        let summary = ParsedSummary {
            request: Some("Wire summary workstream fields".to_string()),
            completed: Some("Added parser and persistence wiring".to_string()),
            decisions: None,
            learned: None,
            next_steps: Some("Run full validation".to_string()),
            preferences: None,
            workstream: Some("Summary WorkStream Persistence".to_string()),
            workstream_progress: Some("ParsedSummary now carries workstream fields".to_string()),
            workstream_next: Some("Open PR for issue 136".to_string()),
            workstream_blockers: Some("Waiting for review".to_string()),
        };

        finalize_summary(
            &mut conn,
            "content-session-1",
            "mem-finalize",
            "test/proj",
            "hash-finalize",
            summary,
        )?;

        let row: (String, Option<String>, Option<String>, Option<String>) = conn.query_row(
            "SELECT title, progress, next_action, blockers
             FROM workstreams
             WHERE project = ?1",
            params!["test/proj"],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(row.0, "Summary WorkStream Persistence");
        assert_eq!(
            row.1.as_deref(),
            Some("ParsedSummary now carries workstream fields")
        );
        assert_eq!(row.2.as_deref(), Some("Open PR for issue 136"));
        assert_eq!(row.3.as_deref(), Some("Waiting for review"));

        let linked_count: i64 = conn.query_row(
            "SELECT COUNT(*)
             FROM workstream_sessions wss
             JOIN workstreams ws ON ws.id = wss.workstream_id
             WHERE wss.memory_session_id = ?1 AND ws.project = ?2",
            params!["mem-finalize", "test/proj"],
            |row| row.get(0),
        )?;
        assert_eq!(linked_count, 1);

        let request: String = conn.query_row(
            "SELECT request FROM session_summaries
             WHERE memory_session_id = ?1 AND project = ?2",
            params!["mem-finalize", "test/proj"],
            |row| row.get(0),
        )?;
        assert_eq!(request, "Wire summary workstream fields");
        Ok(())
    }
}
