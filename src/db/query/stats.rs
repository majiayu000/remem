use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::db::{
    AiUsageBreakdown, AiUsageSourceTotals, AiUsageTotals, DailyAiUsage, WeeklyAiUsage,
};

use super::shared::collect_rows;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemStats {
    pub active_memories: i64,
    pub active_observations: i64,
    pub session_summaries: i64,
    pub raw_messages: i64,
    pub raw_ingest_failures: i64,
    pub raw_ingest_parse_errors: i64,
    pub raw_ingest_insert_errors: i64,
    pub latest_raw_ingest_failure_epoch: Option<i64>,
    pub latest_raw_ingest_failure_kind: Option<String>,
    pub latest_raw_ingest_failure_path: Option<String>,
    pub latest_raw_ingest_failure_message: Option<String>,
    pub captured_events: i64,
    pub pending_extraction_tasks: i64,
    pub processing_extraction_tasks: i64,
    pub failed_extraction_tasks: i64,
    pub oldest_pending_extraction_epoch: Option<i64>,
    pub pending_memory_candidates: i64,
    pub pending_graph_candidates: i64,
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
pub struct MemoryFactsStats {
    pub table_exists: bool,
    pub total: i64,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidatePromotionStat {
    pub review_status: String,
    pub block_reason: Option<String>,
    pub total: i64,
    pub last_7_days: i64,
}

pub fn query_system_stats(conn: &Connection) -> Result<SystemStats> {
    let now = chrono::Utc::now().timestamp();
    let raw_ingest = query_raw_ingest_failure_stats(conn)?;
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
        raw_ingest_failures: raw_ingest.failures,
        raw_ingest_parse_errors: raw_ingest.parse_errors,
        raw_ingest_insert_errors: raw_ingest.insert_errors,
        latest_raw_ingest_failure_epoch: raw_ingest.latest_epoch,
        latest_raw_ingest_failure_kind: raw_ingest.latest_kind,
        latest_raw_ingest_failure_path: raw_ingest.latest_path,
        latest_raw_ingest_failure_message: raw_ingest.latest_message,
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
        pending_graph_candidates: query_pending_graph_candidates(conn)?,
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

pub fn query_memory_facts_stats(conn: &Connection) -> Result<MemoryFactsStats> {
    let table_exists = table_exists(conn, "memory_facts")?;
    let total = if table_exists {
        conn.query_row("SELECT COUNT(*) FROM memory_facts", [], |row| row.get(0))?
    } else {
        0
    };
    Ok(MemoryFactsStats {
        table_exists,
        total,
    })
}

#[derive(Debug, Clone, Default)]
struct RawIngestFailureStats {
    failures: i64,
    parse_errors: i64,
    insert_errors: i64,
    latest_epoch: Option<i64>,
    latest_kind: Option<String>,
    latest_path: Option<String>,
    latest_message: Option<String>,
}

fn query_raw_ingest_failure_stats(conn: &Connection) -> Result<RawIngestFailureStats> {
    if !table_exists(conn, "raw_ingest_failures")? {
        return Ok(RawIngestFailureStats::default());
    }

    let (failures, parse_errors, insert_errors) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(parse_errors), 0), COALESCE(SUM(insert_errors), 0)
         FROM raw_ingest_failures",
        [],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        },
    )?;
    let latest = conn
        .query_row(
            "SELECT created_at_epoch, error_kind, transcript_path, error_message
             FROM raw_ingest_failures
             ORDER BY created_at_epoch DESC, id DESC
             LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;

    let (latest_epoch, latest_kind, latest_path, latest_message) = match latest {
        Some((epoch, kind, path, message)) => (Some(epoch), Some(kind), path, Some(message)),
        None => (None, None, None, None),
    };

    Ok(RawIngestFailureStats {
        failures,
        parse_errors,
        insert_errors,
        latest_epoch,
        latest_kind,
        latest_path,
        latest_message,
    })
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn query_pending_graph_candidates(conn: &Connection) -> Result<i64> {
    if !table_exists(conn, "graph_candidates")? {
        return Ok(0);
    }
    conn.query_row(
		"SELECT COUNT(*) FROM graph_candidates WHERE review_status IN ('pending_review', 'deferred')",
		[],
		|row| row.get(0),
	)
    .map_err(Into::into)
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

pub fn query_candidate_promotion_stats(
    conn: &Connection,
    now_epoch: i64,
) -> Result<Vec<CandidatePromotionStat>> {
    let week_ago = now_epoch - 7 * 24 * 3600;
    let mut stmt = conn.prepare(
        "SELECT review_status,
                auto_promote_block_reason,
                COUNT(*) AS total,
                SUM(CASE WHEN created_at_epoch >= ?1 THEN 1 ELSE 0 END) AS last_7_days
         FROM memory_candidates
         GROUP BY review_status, auto_promote_block_reason
         ORDER BY total DESC, review_status ASC, auto_promote_block_reason ASC",
    )?;
    let rows = stmt.query_map(params![week_ago], |row| {
        Ok(CandidatePromotionStat {
            review_status: row.get(0)?,
            block_reason: row.get(1)?,
            total: row.get(2)?,
            last_7_days: row.get(3)?,
        })
    })?;
    collect_rows(rows)
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

pub fn query_ai_usage_breakdown(
    conn: &Connection,
    since_epoch: Option<i64>,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<AiUsageBreakdown>> {
    if limit <= 0 {
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT project,
                executor,
                usage_source,
                pricing_source,
                COUNT(*),
                COALESCE(SUM(total_tokens), 0),
                COALESCE(SUM(estimated_cost_usd), 0.0)
         FROM ai_usage_events
         WHERE (?1 IS NULL OR created_at_epoch >= ?1)
           AND (?2 IS NULL OR project = ?2)
         GROUP BY project, executor, usage_source, pricing_source
         ORDER BY SUM(estimated_cost_usd) DESC,
                  SUM(total_tokens) DESC,
                  COUNT(*) DESC,
                  project ASC,
                  executor ASC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![since_epoch, project, limit], |row| {
        Ok(AiUsageBreakdown {
            project: row.get(0)?,
            executor: row.get(1)?,
            usage_source: row.get(2)?,
            pricing_source: row.get(3)?,
            calls: row.get(4)?,
            total_tokens: row.get(5)?,
            estimated_cost_usd: row.get(6)?,
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
