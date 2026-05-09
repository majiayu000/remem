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
    pub ready_pending_observations: i64,
    pub delayed_pending_observations: i64,
    pub processing_pending_observations: i64,
    pub expired_processing_pending_observations: i64,
    pub failed_pending_observations: i64,
    pub oldest_ready_pending_epoch: Option<i64>,
    pub pending_jobs: i64,
    pub processing_jobs: i64,
    pub failed_jobs: i64,
    pub stuck_jobs: i64,
    pub worker_daemon_healthy: bool,
    pub worker_heartbeat_owner: Option<String>,
    pub worker_heartbeat_age_secs: Option<i64>,
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
    let now = chrono::Utc::now().timestamp();
    let worker_heartbeat = crate::db::worker::latest_worker_heartbeat(conn)?;
    let worker_heartbeat_age_secs = worker_heartbeat
        .as_ref()
        .map(|heartbeat| now.saturating_sub(heartbeat.updated_at_epoch));
    let worker_daemon_healthy = worker_heartbeat_age_secs
        .map(|age| age <= crate::db::worker::WORKER_HEARTBEAT_HEALTH_SECS)
        .unwrap_or(false);
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
        ready_pending_observations: conn.query_row(
            "SELECT COUNT(*) FROM pending_observations
             WHERE status = 'pending'
               AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?1)
               AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?1)",
            params![now],
            |row| row.get(0),
        )?,
        delayed_pending_observations: conn.query_row(
            "SELECT COUNT(*) FROM pending_observations
             WHERE status = 'pending'
               AND next_retry_epoch IS NOT NULL
               AND next_retry_epoch > ?1",
            params![now],
            |row| row.get(0),
        )?,
        processing_pending_observations: conn.query_row(
            "SELECT COUNT(*) FROM pending_observations WHERE status = 'processing'",
            [],
            |row| row.get(0),
        )?,
        expired_processing_pending_observations: conn.query_row(
            "SELECT COUNT(*) FROM pending_observations
             WHERE status = 'processing'
               AND lease_expires_epoch IS NOT NULL
               AND lease_expires_epoch < ?1",
            params![now],
            |row| row.get(0),
        )?,
        failed_pending_observations: conn.query_row(
            "SELECT COUNT(*) FROM pending_observations WHERE status = 'failed'",
            [],
            |row| row.get(0),
        )?,
        oldest_ready_pending_epoch: conn.query_row(
            "SELECT MIN(created_at_epoch) FROM pending_observations
             WHERE status = 'pending'
               AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?1)
               AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?1)",
            params![now],
            |row| row.get(0),
        )?,
        pending_jobs: conn.query_row("SELECT COUNT(*) FROM jobs WHERE state = 'pending'", [], |row| {
            row.get(0)
        })?,
        processing_jobs: conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE state = 'processing'",
            [],
            |row| row.get(0),
        )?,
        failed_jobs: conn.query_row("SELECT COUNT(*) FROM jobs WHERE state = 'failed'", [], |row| {
            row.get(0)
        })?,
        stuck_jobs: conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE state = 'processing' \
             AND lease_expires_epoch < strftime('%s', 'now')",
            [],
            |row| row.get(0),
        )?,
        worker_daemon_healthy,
        worker_heartbeat_owner: worker_heartbeat.map(|heartbeat| heartbeat.owner),
        worker_heartbeat_age_secs,
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
