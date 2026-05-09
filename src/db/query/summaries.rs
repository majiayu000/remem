use anyhow::Result;
use rusqlite::Connection;

use crate::db::SessionSummary;

use super::shared::{collect_rows, push_project_filter, EPOCH_SECS_ONLY};

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
         WHERE {} AND {} \
         ORDER BY created_at_epoch DESC LIMIT ?{}",
        project_filter, EPOCH_SECS_ONLY, idx
    ))?;

    let refs = crate::db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), |row| {
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
    })?;
    collect_rows(rows)
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
         WHERE memory_session_id = ?1 AND {} AND {} \
         ORDER BY created_at_epoch DESC LIMIT 1",
        project_filter, EPOCH_SECS_ONLY
    ))?;

    let refs = crate::db::to_sql_refs(&param_values);
    let mut rows = stmt.query_map(refs.as_slice(), |row| {
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
    })?;

    match rows.next() {
        Some(Ok(summary)) => Ok(Some(summary)),
        Some(Err(err)) => Err(err.into()),
        None => Ok(None),
    }
}
