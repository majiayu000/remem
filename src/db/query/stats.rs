use anyhow::Result;
use rusqlite::{params, Connection};

use crate::db::{AiUsageSourceTotals, AiUsageTotals, DailyAiUsage, WeeklyAiUsage};

use super::shared::collect_rows;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemStats {
    pub active_memories: i64,
    pub active_observations: i64,
    pub session_summaries: i64,
    pub raw_messages: i64,
    pub captured_events: i64,
    pub pending_extraction_tasks: i64,
    pub processing_extraction_tasks: i64,
    pub failed_extraction_tasks: i64,
    pub oldest_pending_extraction_epoch: Option<i64>,
    pub pending_memory_candidates: i64,
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
    let healthy_worker_heartbeat = crate::db::worker::healthy_worker_heartbeat(
        conn,
        crate::db::worker::WORKER_HEARTBEAT_HEALTH_SECS,
    )?;
    let worker_heartbeat_age_secs = worker_heartbeat
        .as_ref()
        .map(|heartbeat| now.saturating_sub(heartbeat.updated_at_epoch));
    let worker_daemon_healthy = healthy_worker_heartbeat.is_some();
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
        captured_events: conn.query_row("SELECT COUNT(*) FROM captured_events", [], |row| {
            row.get(0)
        })?,
        pending_extraction_tasks: conn.query_row(
            "SELECT COUNT(*) FROM extraction_tasks WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )?,
        processing_extraction_tasks: conn.query_row(
            "SELECT COUNT(*) FROM extraction_tasks WHERE status = 'processing'",
            [],
            |row| row.get(0),
        )?,
        failed_extraction_tasks: conn.query_row(
            "SELECT COUNT(*) FROM extraction_tasks WHERE status = 'failed'",
            [],
            |row| row.get(0),
        )?,
        oldest_pending_extraction_epoch: conn.query_row(
            "SELECT MIN(created_at_epoch) FROM extraction_tasks WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )?,
        pending_memory_candidates: conn.query_row(
            "SELECT COUNT(*) FROM memory_candidates WHERE review_status = 'pending_review'",
            [],
            |row| row.get(0),
        )?,
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

pub fn query_ai_usage_totals(
    conn: &Connection,
    since_epoch: Option<i64>,
    project: Option<&str>,
) -> Result<AiUsageTotals> {
    conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(reasoning_tokens), 0),
                COALESCE(SUM(cache_creation_tokens), 0),
                COALESCE(SUM(cache_read_tokens), 0),
                COALESCE(SUM(total_tokens), 0),
                COALESCE(SUM(estimated_cost_usd), 0.0)
         FROM ai_usage_events
         WHERE (?1 IS NULL OR created_at_epoch >= ?1)
           AND (?2 IS NULL OR project = ?2)",
        params![since_epoch, project],
        |row| {
            Ok(AiUsageTotals {
                calls: row.get(0)?,
                input_tokens: row.get(1)?,
                output_tokens: row.get(2)?,
                reasoning_tokens: row.get(3)?,
                cache_creation_tokens: row.get(4)?,
                cache_read_tokens: row.get(5)?,
                total_tokens: row.get(6)?,
                estimated_cost_usd: row.get(7)?,
            })
        },
    )
    .map_err(Into::into)
}

pub fn query_ai_usage_source_totals(
    conn: &Connection,
    since_epoch: Option<i64>,
    project: Option<&str>,
) -> Result<Vec<AiUsageSourceTotals>> {
    let mut stmt = conn.prepare(
        "SELECT usage_source,
                pricing_source,
                COUNT(*),
                COALESCE(SUM(total_tokens), 0),
                COALESCE(SUM(estimated_cost_usd), 0.0)
         FROM ai_usage_events
         WHERE (?1 IS NULL OR created_at_epoch >= ?1)
           AND (?2 IS NULL OR project = ?2)
         GROUP BY usage_source, pricing_source
         ORDER BY SUM(total_tokens) DESC",
    )?;
    let rows = stmt.query_map(params![since_epoch, project], |row| {
        Ok(AiUsageSourceTotals {
            usage_source: row.get(0)?,
            pricing_source: row.get(1)?,
            calls: row.get(2)?,
            total_tokens: row.get(3)?,
            estimated_cost_usd: row.get(4)?,
        })
    })?;
    collect_rows(rows)
}

pub fn query_daily_ai_usage(
    conn: &Connection,
    since_epoch: i64,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<DailyAiUsage>> {
    let mut stmt = conn.prepare(
        "SELECT strftime('%Y-%m-%d', created_at_epoch, 'unixepoch') AS day,
                COUNT(*),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(reasoning_tokens), 0),
                COALESCE(SUM(cache_creation_tokens), 0),
                COALESCE(SUM(cache_read_tokens), 0),
                COALESCE(SUM(total_tokens), 0),
                COALESCE(SUM(estimated_cost_usd), 0.0)
         FROM ai_usage_events
         WHERE created_at_epoch >= ?1
           AND (?2 IS NULL OR project = ?2)
         GROUP BY day
         ORDER BY day DESC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![since_epoch, project, limit], |row| {
        Ok(DailyAiUsage {
            day: row.get(0)?,
            calls: row.get(1)?,
            input_tokens: row.get(2)?,
            output_tokens: row.get(3)?,
            reasoning_tokens: row.get(4)?,
            cache_creation_tokens: row.get(5)?,
            cache_read_tokens: row.get(6)?,
            total_tokens: row.get(7)?,
            estimated_cost_usd: row.get(8)?,
        })
    })?;
    collect_rows(rows)
}

pub fn query_weekly_ai_usage(
    conn: &Connection,
    since_epoch: i64,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<WeeklyAiUsage>> {
    let mut stmt = conn.prepare(
        "SELECT strftime('%Y-W%W', created_at_epoch, 'unixepoch') AS week,
                COUNT(*),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(reasoning_tokens), 0),
                COALESCE(SUM(cache_creation_tokens), 0),
                COALESCE(SUM(cache_read_tokens), 0),
                COALESCE(SUM(total_tokens), 0),
                COALESCE(SUM(estimated_cost_usd), 0.0)
         FROM ai_usage_events
         WHERE created_at_epoch >= ?1
           AND (?2 IS NULL OR project = ?2)
         GROUP BY week
         ORDER BY week DESC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![since_epoch, project, limit], |row| {
        Ok(WeeklyAiUsage {
            week: row.get(0)?,
            calls: row.get(1)?,
            input_tokens: row.get(2)?,
            output_tokens: row.get(3)?,
            reasoning_tokens: row.get(4)?,
            cache_creation_tokens: row.get(5)?,
            cache_read_tokens: row.get(6)?,
            total_tokens: row.get(7)?,
            estimated_cost_usd: row.get(8)?,
        })
    })?;
    collect_rows(rows)
}

#[cfg(test)]
mod tests;
