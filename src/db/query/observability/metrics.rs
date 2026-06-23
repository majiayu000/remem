use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::{params, Connection};

use super::super::shared::collect_rows;
use super::checks::observability_checks;
use super::types::{
    CaptureObservabilityMetrics, ContextInjectionObservabilityMetrics, CountBucket,
    ObservabilityMetrics, ObservabilityReport, PromotionObservabilityMetrics,
    QueueObservabilityMetrics, StalenessObservabilityMetrics, TemporalFactObservabilityMetrics,
    UsageFeedbackObservabilityMetrics, WorkerObservabilityMetrics,
    CURRENT_MEMORY_CONTRACT_SPEC_PATH, OBSERVABILITY_SCHEMA_VERSION,
};
use crate::memory::{
    map_memory_row_pub, memory_current_filter_sql, memory_staleness_labels_for_memories_lossy,
    memory_state_key_current_filter_sql, MEMORY_COLS,
};

pub fn query_observability_report(
    conn: &Connection,
    generated_at_epoch: i64,
) -> Result<ObservabilityReport> {
    let stats = crate::db::query_system_stats(conn)?;
    let capture = capture_metrics(&stats);
    let promotion = promotion_metrics(&stats);
    let context_injection = query_context_injection_metrics(conn)?;
    let usage_feedback = query_usage_feedback_metrics(conn)?;
    let temporal_facts = query_temporal_fact_metrics(conn)?;
    let staleness = query_staleness_metrics(conn, generated_at_epoch)?;
    let queue = queue_metrics(&stats);
    let worker = worker_metrics(&stats);
    let checks = observability_checks(
        &capture,
        &promotion,
        &context_injection,
        &usage_feedback,
        &temporal_facts,
        &staleness,
        &queue,
        &worker,
    );

    Ok(ObservabilityReport {
        schema_version: OBSERVABILITY_SCHEMA_VERSION,
        generated_at_epoch,
        spec_path: CURRENT_MEMORY_CONTRACT_SPEC_PATH,
        checks,
        metrics: ObservabilityMetrics {
            capture,
            promotion,
            context_injection,
            usage_feedback,
            temporal_facts,
            staleness,
            queue,
            worker,
        },
    })
}

fn capture_metrics(stats: &crate::db::SystemStats) -> CaptureObservabilityMetrics {
    CaptureObservabilityMetrics {
        captured_events: stats.captured_events,
        capture_drop_events: stats.capture_drop_events,
        actionable_capture_drops: stats.actionable_capture_drops,
        unrecovered_capture_spills: stats.unrecovered_capture_spills,
        pending_extraction_tasks: stats.pending_extraction_tasks,
        processing_extraction_tasks: stats.processing_extraction_tasks,
        expired_processing_extraction_tasks: stats.expired_processing_extraction_tasks,
        failed_extraction_tasks: stats.failed_extraction_tasks,
    }
}

fn promotion_metrics(stats: &crate::db::SystemStats) -> PromotionObservabilityMetrics {
    PromotionObservabilityMetrics {
        observations: stats.total_observations,
        candidates: stats.total_memory_candidates,
        promoted: stats.promoted_memory_candidates,
        pending_review: stats.pending_review_memory_candidates,
        candidate_rate_percent: percent(stats.total_memory_candidates, stats.total_observations),
        promoted_rate_percent: percent(
            stats.promoted_memory_candidates,
            stats.total_memory_candidates,
        ),
    }
}

fn queue_metrics(stats: &crate::db::SystemStats) -> QueueObservabilityMetrics {
    QueueObservabilityMetrics {
        pending_observations: stats.pending_observations,
        ready_pending_observations: stats.ready_pending_observations,
        delayed_pending_observations: stats.delayed_pending_observations,
        processing_pending_observations: stats.processing_pending_observations,
        expired_processing_pending_observations: stats.expired_processing_pending_observations,
        failed_pending_observations: stats.failed_pending_observations,
        pending_jobs: stats.pending_jobs,
        processing_jobs: stats.processing_jobs,
        failed_jobs: stats.failed_jobs,
        stuck_jobs: stats.stuck_jobs,
        retryable_extraction_replay_ranges: stats.retryable_extraction_replay_ranges,
    }
}

fn worker_metrics(stats: &crate::db::SystemStats) -> WorkerObservabilityMetrics {
    WorkerObservabilityMetrics {
        daemon_healthy: stats.worker_daemon_healthy,
        heartbeat_age_secs: stats.worker_heartbeat_age_secs,
        heartbeat_owner_present: stats.worker_heartbeat_owner.is_some(),
    }
}

fn query_context_injection_metrics(
    conn: &Connection,
) -> Result<ContextInjectionObservabilityMetrics> {
    let output_table_exists = table_exists(conn, "context_injections")?;
    let item_table_exists = table_exists(conn, "context_injection_items")?;
    let (output_rows, output_emit_count, output_suppress_count, output_modes) =
        if output_table_exists {
            let totals = conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(emit_count), 0), COALESCE(SUM(suppress_count), 0)
                 FROM context_injections",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            (
                totals.0,
                totals.1,
                totals.2,
                grouped_counts(conn, "context_injections", "output_mode")?,
            )
        } else {
            (0, 0, 0, Vec::new())
        };

    let item_metrics = if item_table_exists {
        Some((
            table_count(conn, "context_injection_items")?,
            grouped_counts(conn, "context_injection_items", "status")?,
            grouped_counts(conn, "context_injection_items", "channel")?,
            grouped_counts(conn, "context_injection_items", "drop_reason")?,
            grouped_staleness_token_counts(conn, "source_anchor")?,
            grouped_staleness_token_counts(conn, "staleness")?,
        ))
    } else {
        None
    };
    let (item_rows, item_statuses, item_channels, item_drop_reasons, anchors, ages) =
        item_metrics.unwrap_or_default();

    Ok(ContextInjectionObservabilityMetrics {
        output_table_exists,
        item_table_exists,
        output_rows,
        output_emit_count,
        output_suppress_count,
        output_modes,
        item_rows,
        item_statuses,
        item_channels,
        item_drop_reasons,
        item_staleness_source_anchors: anchors,
        item_staleness_ages: ages,
    })
}

fn query_usage_feedback_metrics(conn: &Connection) -> Result<UsageFeedbackObservabilityMetrics> {
    let citation_table_exists = table_exists(conn, "memory_citation_events")?;
    let usage_table_exists = table_exists(conn, "memory_usage_events")?;
    let mut metrics = UsageFeedbackObservabilityMetrics {
        citation_table_exists,
        usage_table_exists,
        ..UsageFeedbackObservabilityMetrics::default()
    };

    if citation_table_exists {
        let (
            citation_events,
            citation_line_present_events,
            matched_events,
            inserted_events,
            no_citation_events,
            unmatched_events,
        ) = conn.query_row(
            "SELECT
                 COUNT(*),
                 COALESCE(SUM(CASE WHEN citation_line_present > 0 THEN 1 ELSE 0 END), 0),
                 COALESCE(SUM(CASE WHEN status = 'matched' THEN 1 ELSE 0 END), 0),
                 COALESCE(SUM(CASE WHEN inserted_count > 0 THEN 1 ELSE 0 END), 0),
                 COALESCE(SUM(CASE WHEN status = 'no_citation' THEN 1 ELSE 0 END), 0),
                 COALESCE(SUM(CASE WHEN status = 'unmatched' THEN 1 ELSE 0 END), 0)
             FROM memory_citation_events",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )?;
        metrics.citation_events = citation_events;
        metrics.citation_line_present_events = citation_line_present_events;
        metrics.matched_events = matched_events;
        metrics.inserted_events = inserted_events;
        metrics.no_citation_events = no_citation_events;
        metrics.unmatched_events = unmatched_events;
    }

    if usage_table_exists {
        metrics.usage_events = table_count(conn, "memory_usage_events")?;
    }

    Ok(metrics)
}

fn query_temporal_fact_metrics(conn: &Connection) -> Result<TemporalFactObservabilityMetrics> {
    let stats = crate::db::query_memory_facts_stats(conn)?;
    if !stats.table_exists {
        return Ok(TemporalFactObservabilityMetrics::default());
    }

    let now = chrono::Utc::now().timestamp();
    let invalidated_rows = if column_exists(conn, "memory_facts", "invalidated_at_epoch")? {
        conn.query_row(
            "SELECT COUNT(*) FROM memory_facts WHERE invalidated_at_epoch IS NOT NULL",
            [],
            |row| row.get(0),
        )?
    } else {
        0
    };
    let expired_rows = conn.query_row(
        "SELECT COUNT(*) FROM memory_facts WHERE valid_to_epoch IS NOT NULL AND valid_to_epoch < ?1",
        params![now],
        |row| row.get(0),
    )?;
    let orphan_source_memory_rows = if table_exists(conn, "memories")? {
        conn.query_row(
            "SELECT COUNT(*) FROM memory_facts f
             LEFT JOIN memories m ON m.id = f.source_memory_id
             WHERE f.source_memory_id IS NOT NULL AND m.id IS NULL",
            [],
            |row| row.get(0),
        )?
    } else {
        0
    };
    let unlinked_source_rows = conn.query_row(
        "SELECT COUNT(*) FROM memory_facts
         WHERE source_memory_id IS NULL
           AND source_observation_id IS NULL
           AND trim(COALESCE(source_event_ids, '[]')) IN ('', '[]')",
        [],
        |row| row.get(0),
    )?;

    Ok(TemporalFactObservabilityMetrics {
        table_exists: true,
        total_rows: stats.total,
        retrieval_eligible_rows: stats.retrieval_eligible,
        invalidated_rows,
        expired_rows,
        orphan_source_memory_rows,
        unlinked_source_rows,
    })
}

fn query_staleness_metrics(
    conn: &Connection,
    now_epoch: i64,
) -> Result<StalenessObservabilityMetrics> {
    if !table_exists(conn, "memories")? {
        return Ok(StalenessObservabilityMetrics::default());
    }

    let active_filter = memory_current_filter_sql("m.status", "m.expires_at_epoch", false);
    let state_filter = memory_state_key_current_filter_sql("m");
    let sql =
        format!("SELECT {MEMORY_COLS} FROM memories m WHERE {active_filter} AND {state_filter}");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], map_memory_row_pub)?;
    let memories = collect_rows(rows)?;
    let mut error_count = 0_i64;
    let labels = memory_staleness_labels_for_memories_lossy(conn, &memories, now_epoch, |_, _| {
        error_count += 1;
    })?;
    let mut source_anchors = BTreeMap::new();
    let mut ages = BTreeMap::new();
    for memory in &memories {
        if let Some(label) = labels.get(&memory.id) {
            increment(&mut source_anchors, label.source_anchor.as_str());
            increment(&mut ages, label.age);
        }
    }

    Ok(StalenessObservabilityMetrics {
        memory_table_exists: true,
        total_memories: memories.len() as i64,
        source_anchors: buckets(source_anchors),
        ages: buckets(ages),
        error_count,
    })
}

fn grouped_counts(conn: &Connection, table: &str, column: &str) -> Result<Vec<CountBucket>> {
    let sql = format!(
        "SELECT COALESCE({column}, 'none') AS value, COUNT(*)
         FROM {table}
         GROUP BY value
         ORDER BY COUNT(*) DESC, value ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(CountBucket {
            value: row.get::<_, String>(0)?,
            count: row.get(1)?,
        })
    })?;
    collect_rows(rows)
}

fn grouped_staleness_token_counts(conn: &Connection, token: &str) -> Result<Vec<CountBucket>> {
    let mut stmt = conn.prepare("SELECT staleness FROM context_injection_items")?;
    let rows = stmt.query_map([], |row| row.get::<_, Option<String>>(0))?;
    let mut counts = BTreeMap::new();
    for row in rows {
        let Some(staleness) = row? else {
            increment(&mut counts, "unknown");
            continue;
        };
        increment(&mut counts, extract_staleness_token(&staleness, token));
    }
    Ok(buckets(counts))
}

fn extract_staleness_token<'a>(staleness: &'a str, token: &str) -> &'a str {
    for part in staleness.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix(&format!("{token}=")) {
            return value.trim();
        }
    }
    "unknown"
}

fn table_count(conn: &Connection, table: &str) -> Result<i64> {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .map_err(Into::into)
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    crate::retrieval::temporal::sqlite_table_exists(conn, table)
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    if !table_exists(conn, table)? {
        return Ok(false);
    }
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        params![table, column],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn percent(numerator: i64, denominator: i64) -> f64 {
    if denominator <= 0 {
        0.0
    } else {
        (numerator as f64 * 100.0) / denominator as f64
    }
}

fn increment(counts: &mut BTreeMap<String, i64>, value: &str) {
    *counts.entry(value.to_string()).or_default() += 1;
}

fn buckets(counts: BTreeMap<String, i64>) -> Vec<CountBucket> {
    counts
        .into_iter()
        .map(|(value, count)| CountBucket { value, count })
        .collect()
}
