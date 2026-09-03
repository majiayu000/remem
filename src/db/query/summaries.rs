use anyhow::Result;
use rusqlite::Connection;

use crate::db::summary_poisoning::{summary_injectable, NOT_QUARANTINED_SQL};
use crate::db::SessionSummary;

use super::shared::{collect_rows, push_project_filter, EPOCH_SECS_ONLY};

fn summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionSummary> {
    Ok(SessionSummary {
        id: row.get(0)?,
        memory_session_id: row.get(1)?,
        request: row.get(2)?,
        completed: row.get(3)?,
        decisions: row.get(4)?,
        learned: row.get(5)?,
        next_steps: row.get(6)?,
        preferences: row.get(7)?,
        created_at: row.get(8)?,
        created_at_epoch: row.get(9)?,
        project: row.get(10)?,
    })
}

fn summary_passes_poisoning_gate(conn: &Connection, summary: &SessionSummary, sink: &str) -> bool {
    summary_injectable(
        conn,
        summary.id,
        &[
            ("request", summary.request.as_deref()),
            ("completed", summary.completed.as_deref()),
            ("decisions", summary.decisions.as_deref()),
            ("learned", summary.learned.as_deref()),
            ("next_steps", summary.next_steps.as_deref()),
            ("preferences", summary.preferences.as_deref()),
        ],
        sink,
    )
}

pub fn query_summaries(
    conn: &Connection,
    project: &str,
    limit: i64,
) -> Result<Vec<SessionSummary>> {
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (project_filter, idx) = push_project_filter("project", project, 1, &mut param_values);
    param_values.push(Box::new(limit));

    let mut stmt = conn.prepare(&format!(
        "SELECT id, memory_session_id, request, completed, decisions, learned, \
         next_steps, preferences, created_at, created_at_epoch, project \
         FROM session_summaries \
         WHERE {} AND {} AND {} \
         ORDER BY created_at_epoch DESC LIMIT ?{}",
        project_filter, EPOCH_SECS_ONLY, NOT_QUARANTINED_SQL, idx
    ))?;

    let refs = crate::db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), summary_from_row)?;
    let mut summaries = collect_rows(rows)?;
    summaries.retain(|summary| summary_passes_poisoning_gate(conn, summary, "query_summaries"));
    Ok(summaries)
}

pub fn get_summary_by_session(
    conn: &Connection,
    memory_session_id: &str,
    project: &str,
) -> Result<Option<SessionSummary>> {
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(memory_session_id.to_string()));
    let (project_filter, _) = push_project_filter("project", project, 2, &mut param_values);

    let mut stmt = conn.prepare(&format!(
        "SELECT id, memory_session_id, request, completed, decisions, learned, \
         next_steps, preferences, created_at, created_at_epoch, project \
         FROM session_summaries \
         WHERE memory_session_id = ?1 AND {} AND {} AND {} \
         ORDER BY created_at_epoch DESC LIMIT 1",
        project_filter, EPOCH_SECS_ONLY, NOT_QUARANTINED_SQL
    ))?;

    let refs = crate::db::to_sql_refs(&param_values);
    let mut rows = stmt.query_map(refs.as_slice(), summary_from_row)?;

    match rows.next() {
        Some(Ok(summary)) => {
            if summary_passes_poisoning_gate(conn, &summary, "get_summary_by_session") {
                Ok(Some(summary))
            } else {
                Ok(None)
            }
        }
        Some(Err(err)) => Err(err.into()),
        None => Ok(None),
    }
}

pub fn get_summaries_by_ids(
    conn: &Connection,
    ids: &[i64],
    project: Option<&str>,
) -> Result<Vec<SessionSummary>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders = (1..=ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>();
    let mut parameters = ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect::<Vec<_>>();
    let mut conditions = vec![
        format!("id IN ({})", placeholders.join(", ")),
        EPOCH_SECS_ONLY.to_string(),
        NOT_QUARANTINED_SQL.to_string(),
    ];
    if let Some(project) = project {
        let (project_filter, _) = crate::project_alias::push_project_value_filter(
            conn,
            "project",
            project,
            ids.len() + 1,
            &mut parameters,
        )?;
        conditions.push(project_filter);
    }

    let mut stmt = conn.prepare(&format!(
        "SELECT id, memory_session_id, request, completed, decisions, learned, \
         next_steps, preferences, COALESCE(created_at, datetime(created_at_epoch, 'unixepoch')), \
         created_at_epoch, project \
         FROM session_summaries WHERE {} ORDER BY created_at_epoch DESC",
        conditions.join(" AND ")
    ))?;
    let refs = crate::db::to_sql_refs(&parameters);
    let rows = stmt.query_map(refs.as_slice(), summary_from_row)?;
    let mut summaries = collect_rows(rows)?;
    summaries
        .retain(|summary| summary_passes_poisoning_gate(conn, summary, "get_summaries_by_ids"));
    Ok(summaries)
}
