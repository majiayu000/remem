use anyhow::Result;
use rusqlite::Connection;

use crate::cli::types::{RawAction, RawRole};
use crate::memory::raw_archive::{
    build_sessions_json, list_sessions, RawMessage, RawSearchRequest, RawSessionQuery,
    RawSessionSummary,
};
use crate::memory::raw_query::{
    build_raw_search_json, parse_time_lower_bound, parse_time_upper_bound,
    query_raw_session_messages, RawSessionMessagesRequest,
};
use crate::{db, memory::raw_archive::search_raw_messages};

use super::show::format_memory_timestamp;

pub(in crate::cli) fn run_raw(action: RawAction) -> Result<()> {
    match action {
        RawAction::Search {
            query,
            project,
            branch,
            role,
            limit,
            offset,
            since,
            until,
            json,
        } => run_raw_search(
            &query,
            project.as_deref(),
            branch.as_deref(),
            role,
            limit,
            offset,
            since.as_deref().map(parse_time_lower_bound).transpose()?,
            until.as_deref().map(parse_time_upper_bound).transpose()?,
            json,
        ),
        RawAction::Sessions {
            since,
            until,
            project,
            sample,
            json,
        } => run_raw_sessions(
            since.as_deref().map(parse_time_lower_bound).transpose()?,
            until.as_deref().map(parse_time_upper_bound).transpose()?,
            project.as_deref(),
            sample,
            json,
        ),
        RawAction::Messages {
            source_root,
            project,
            session_id,
            limit,
            cursor,
            json,
        } => run_raw_messages(
            &source_root,
            &project,
            &session_id,
            limit,
            cursor.as_deref(),
            json,
        ),
        RawAction::Reconcile {
            since,
            until,
            roots,
            json,
        } => run_raw_reconcile(
            parse_time_lower_bound(&since)?,
            parse_time_upper_bound(&until)?,
            &roots,
            json,
        ),
    }
}

fn run_raw_messages(
    source_root: &str,
    project: &str,
    session_id: &str,
    limit: i64,
    cursor: Option<&str>,
    json: bool,
) -> Result<()> {
    let conn = db::open_db_read_only_current()?;
    let output = query_raw_session_messages(
        &conn,
        &RawSessionMessagesRequest {
            source_root: source_root.to_string(),
            project: project.to_string(),
            session_id: session_id.to_string(),
            limit,
            cursor: cursor.map(str::to_string),
        },
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }
    println!(
        "{} raw messages for [{source_root}] {project} / {session_id} (order={}, has_more={})",
        output.count, output.order, output.has_more
    );
    if let Some(cursor) = output.next_cursor {
        println!("Next: remem raw messages --source-root <LABEL> --project <PROJECT> --session-id <SESSION_ID> --cursor {cursor}");
    }
    Ok(())
}

fn run_raw_reconcile(
    since_epoch: i64,
    until_epoch: i64,
    root_specs: &[String],
    json: bool,
) -> Result<()> {
    let mut roots = crate::ingest::sessions::default_scan_roots();
    roots.extend(
        root_specs
            .iter()
            .map(|spec| crate::ingest::sessions::ScanRoot::parse(spec))
            .collect::<Result<Vec<_>>>()?,
    );
    let conn = db::open_db_read_only_current()?;
    let report = crate::memory::raw_reconcile::reconcile_raw_archive(
        &conn,
        &roots,
        since_epoch,
        until_epoch,
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!(
            "{}",
            crate::memory::raw_reconcile::render_reconcile_human(&report)
        );
    }
    ensure_reconcile_parity(report.parity)?;
    Ok(())
}

fn ensure_reconcile_parity(parity: bool) -> Result<()> {
    if !parity {
        anyhow::bail!(
            "raw reconciliation found strict parity failures; inspect the aggregate report"
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_raw_search(
    query: &str,
    project: Option<&str>,
    branch: Option<&str>,
    role: Option<RawRole>,
    limit: i64,
    offset: i64,
    since_epoch: Option<i64>,
    until_epoch: Option<i64>,
    json: bool,
) -> Result<()> {
    let conn = db::open_db_read_only_current()?;
    let normalized_limit = limit.max(1);
    let normalized_offset = offset.max(0);
    let request = build_raw_search_request(
        query,
        project,
        branch,
        role.map(RawRole::as_str),
        normalized_limit.saturating_add(1),
        normalized_offset,
        since_epoch,
        until_epoch,
    );
    let mut rows = search_raw_archive(&conn, &request)?;
    let has_more = rows.len() as i64 > normalized_limit;
    rows.truncate(normalized_limit as usize);

    if json {
        let output = build_raw_search_json(
            query,
            project,
            branch,
            role.map(RawRole::as_str),
            normalized_limit,
            normalized_offset,
            since_epoch,
            until_epoch,
            has_more,
            &rows,
        );
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    print!(
        "{}",
        render_raw_search_results(&rows, normalized_offset, normalized_limit, has_more)
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_raw_search_request(
    query: &str,
    project: Option<&str>,
    branch: Option<&str>,
    role: Option<&str>,
    limit: i64,
    offset: i64,
    since_epoch: Option<i64>,
    until_epoch: Option<i64>,
) -> RawSearchRequest {
    RawSearchRequest {
        query: query.to_string(),
        project: project.map(str::to_string),
        branch: branch.map(str::to_string),
        role: role.map(str::to_string),
        limit,
        offset,
        since_epoch,
        until_epoch,
    }
}

pub(super) fn run_raw_sessions(
    since_epoch: Option<i64>,
    until_epoch: Option<i64>,
    project: Option<&str>,
    sample: i64,
    json: bool,
) -> Result<()> {
    let conn = db::open_db_read_only_current()?;
    let query = RawSessionQuery {
        since_epoch,
        until_epoch,
        project: project.map(str::to_string),
        sample_user_messages: sample.max(0),
    };
    let sessions = list_sessions(&conn, &query)?;

    if json {
        let output = build_sessions_json(&query, sessions);
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }
    print!("{}", render_raw_sessions(&sessions));
    Ok(())
}

pub(super) fn render_raw_sessions(sessions: &[RawSessionSummary]) -> String {
    let mut output = String::new();
    if sessions.is_empty() {
        output.push_str("No sessions with raw messages in this window.\n");
        return output;
    }
    output.push_str(&format!("{} sessions in window:\n\n", sessions.len()));
    for session in sessions {
        output.push_str(&format!(
            "  [{}] {} | {} | {} .. {} | {} messages\n",
            session.source_root,
            session.project,
            session.session_id,
            format_memory_timestamp(session.first_epoch),
            format_memory_timestamp(session.last_epoch),
            session.message_count
        ));
        for sample in &session.user_message_samples {
            output.push_str(&format!("      user: {}\n", sample.replace('\n', " ")));
        }
    }
    output
}

pub(super) fn search_raw_archive(
    conn: &Connection,
    request: &RawSearchRequest,
) -> Result<Vec<RawMessage>> {
    search_raw_messages(conn, request)
}

pub(super) fn render_raw_search_results(
    rows: &[RawMessage],
    offset: i64,
    limit: i64,
    has_more: bool,
) -> String {
    let mut output = String::new();
    if rows.is_empty() {
        output.push_str("No raw archive rows found.\n");
        output.push_str(
            "Curated search may still have promoted memories: remem search \"<query>\".\n",
        );
        return output;
    }

    output.push_str("Raw archive rows (not curated memories):\n\n");
    for row in rows {
        output.push_str(&format_raw_row(row));
    }

    output.push_str("\nNext:\n");
    output.push_str("  raw rows are captured chat turns, not curated memories.\n");
    output.push_str("  promote durable conclusions with review/save_memory.\n");
    if has_more {
        output.push_str(&format!(
            "  remem raw search \"<query>\" --offset {}\n",
            offset.max(0) + limit.max(1)
        ));
    }
    output
}

fn format_raw_row(row: &RawMessage) -> String {
    let branch = row
        .branch
        .as_deref()
        .map(|branch| format!(" | branch={branch}"))
        .unwrap_or_default();
    let cwd = row
        .cwd
        .as_deref()
        .map(|cwd| format!(" | cwd={cwd}"))
        .unwrap_or_default();
    let preview = preview_raw_content(row);
    let mut output = format!(
        "  [raw:{}] {} | {} | {} | source={}{}{}\n",
        row.id,
        row.role,
        row.project,
        format_memory_timestamp(row.created_at_epoch),
        row.source,
        branch,
        cwd
    );
    if !preview.is_empty() {
        output.push_str(&format!("      {}\n", preview));
    }
    output
}

fn preview_raw_content(row: &RawMessage) -> String {
    row.content
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(200)
        .collect()
}

#[cfg(test)]
mod reconcile_exit_tests {
    use super::ensure_reconcile_parity;

    #[test]
    fn every_non_parity_report_produces_a_cli_error() {
        assert!(ensure_reconcile_parity(false).is_err());
        assert!(ensure_reconcile_parity(true).is_ok());
    }
}

#[cfg(test)]
mod lock_contention_tests {
    use anyhow::Result;

    use super::{run_raw_search, run_raw_sessions};

    #[test]
    fn raw_search_and_sessions_actions_succeed_during_normal_write_contention() -> Result<()> {
        let _data_dir = crate::db::test_support::ScopedTestDataDir::new("raw-actions-write-lock");
        let writer = crate::db::open_db()?;
        crate::memory::raw_archive::insert_raw_message(
            &writer,
            "lock-session",
            "lock-project",
            crate::memory::raw_archive::ROLE_USER,
            "visible during writer lock",
            crate::memory::raw_archive::SOURCE_MANUAL,
            None,
            None,
        )?;
        writer.execute_batch("BEGIN IMMEDIATE")?;

        let search_result = run_raw_search("visible", None, None, None, 20, 0, None, None, true);
        let sessions_result = run_raw_sessions(None, None, None, 0, true);
        writer.execute_batch("ROLLBACK")?;

        search_result?;
        sessions_result?;
        Ok(())
    }
}
