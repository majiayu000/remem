use anyhow::Result;
use rusqlite::{params, Connection};

use super::shared::collect_rows;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemStats {
    pub active_memories: i64,
    pub active_observations: i64,
    pub session_summaries: i64,
    pub raw_messages: i64,
    pub pending_observations: i64,
    pub failed_pending_observations: i64,
    pub stuck_jobs: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyActivityStats {
    pub memories: i64,
    pub observations: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectCount {
    pub project: String,
    pub count: i64,
}

pub fn query_system_stats(conn: &Connection) -> Result<SystemStats> {
    Ok(SystemStats {
        active_memories: conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE status = 'active'",
            [],
            |row| row.get(0),
        )?,
        active_observations: conn.query_row(
            "SELECT COUNT(*) FROM observations WHERE status = 'active'",
            [],
            |row| row.get(0),
        )?,
        session_summaries: conn.query_row("SELECT COUNT(*) FROM session_summaries", [], |row| {
            row.get(0)
        })?,
        raw_messages: conn.query_row("SELECT COUNT(*) FROM raw_messages", [], |row| row.get(0))?,
        pending_observations: conn.query_row(
            "SELECT COUNT(*) FROM pending_observations WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )?,
        failed_pending_observations: conn.query_row(
            "SELECT COUNT(*) FROM pending_observations WHERE status = 'failed'",
            [],
            |row| row.get(0),
        )?,
        stuck_jobs: conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE state = 'running' \
             AND lease_expires_epoch < strftime('%s', 'now')",
            [],
            |row| row.get(0),
        )?,
    })
}

pub fn query_daily_activity_stats(
    conn: &Connection,
    since_epoch: i64,
) -> Result<DailyActivityStats> {
    Ok(DailyActivityStats {
        memories: conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE created_at_epoch >= ?1",
            params![since_epoch],
            |row| row.get(0),
        )?,
        observations: conn.query_row(
            "SELECT COUNT(*) FROM observations WHERE created_at_epoch >= ?1",
            params![since_epoch],
            |row| row.get(0),
        )?,
    })
}

pub fn query_top_projects(conn: &Connection, limit: i64) -> Result<Vec<ProjectCount>> {
    let mut stmt = conn.prepare(
        "SELECT project, COUNT(*) as cnt FROM memories WHERE status = 'active' \
         GROUP BY project ORDER BY cnt DESC, project ASC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok(ProjectCount {
            project: row.get(0)?,
            count: row.get(1)?,
        })
    })?;
    collect_rows(rows)
}

#[cfg(test)]
mod tests;
