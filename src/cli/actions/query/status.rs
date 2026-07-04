use anyhow::{Context, Result};
use serde::Serialize;

use crate::db;
use crate::doctor::health_action::{
    queue_actions_with_replay, render_action_block, worker_once_fallback_human,
};

pub(in crate::cli) fn run_status(json: bool) -> Result<()> {
    let report = load_status_report()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    print_status_report(&report);
    Ok(())
}

fn load_status_report() -> Result<StatusReport> {
    let db_path = db::db_path();
    ensure_status_database_can_migrate(&db_path)?;
    let conn = db::open_db()?;
    let db_size = std::fs::metadata(&db_path)
        .with_context(|| format!("failed to stat database path {}", db_path.display()))?
        .len();
    let version = crate::build_info::version_label();
    let stats = db::query_system_stats(&conn)?;

    let today_start = chrono::Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|dt| dt.and_utc().timestamp())
        .unwrap_or(0);
    let daily_stats = db::query_daily_activity_stats(&conn, today_start)?;
    let top_projects = db::query_top_projects(&conn, 5)?;
    let now = chrono::Utc::now().timestamp();
    let candidate_promotion = db::query_candidate_promotion_stats(&conn, now)?;
    let latest_session_memory_spend = db::query_latest_session_memory_spend(&conn)?;
    let usage_feedback = crate::memory::usage::query_memory_usage_feedback_stats(&conn)?;
    let embedding_provider = crate::retrieval::embedding::embedding_provider_status()?;
    let embedding_coverage =
        crate::retrieval::vector::active_embedding_coverage_for_status(&conn, &embedding_provider)?;

    Ok(StatusReport {
        version,
        database: StatusDatabase {
            path: db_path.display().to_string(),
            size_bytes: db_size,
            size_mb: db_size as f64 / 1_048_576.0,
        },
        totals: StatusTotals {
            memories: stats.active_memories,
            observations: stats.active_observations,
            sessions: stats.session_summaries,
            raw_messages: stats.raw_messages,
        },
        embedding: EmbeddingStatus {
            configured_provider: embedding_provider.configured_provider,
            fallback_provider: embedding_provider.fallback_provider,
            active_provider: embedding_provider.active_provider,
            active_model_id: embedding_provider.active_model_id,
            degraded: embedding_provider.degraded,
            disabled: embedding_provider.disabled,
            unavailable_reason: embedding_provider.unavailable_reason,
            degradation_reason: embedding_provider.degradation_reason,
            coverage: EmbeddingCoverageStatus {
                embedded: embedding_coverage.embedded,
                total: embedding_coverage.total,
                percent: embedding_coverage.percent,
                mixed_profile_count: embedding_coverage.mixed_profile_count,
            },
        },
        raw_archive: RawArchiveStatus {
            messages: stats.raw_messages,
            ingest_failures: stats.raw_ingest_failures,
            parse_errors: stats.raw_ingest_parse_errors,
            insert_errors: stats.raw_ingest_insert_errors,
            latest_failure_epoch: stats.latest_raw_ingest_failure_epoch,
            latest_failure_age_secs: stats
                .latest_raw_ingest_failure_epoch
                .map(|epoch| now.saturating_sub(epoch)),
            latest_failure_kind: stats.latest_raw_ingest_failure_kind,
            latest_failure_path: stats.latest_raw_ingest_failure_path,
            latest_failure_message: stats.latest_raw_ingest_failure_message,
        },
        capture_pipeline: CapturePipelineStatus {
            captured: stats.captured_events,
            dropped: stats.capture_drop_events,
            unrecovered_spills: stats.unrecovered_capture_spills,
            latest_drop_epoch: stats.latest_capture_drop_epoch,
            latest_drop_age_secs: stats
                .latest_capture_drop_epoch
                .map(|epoch| now.saturating_sub(epoch)),
            latest_drop_reason: stats.latest_capture_drop_reason,
            latest_drop_detail: stats.latest_capture_drop_detail,
            extract_todo: stats.pending_extraction_tasks,
            extract_running: stats.processing_extraction_tasks,
            extract_expired: stats.expired_processing_extraction_tasks,
            extract_failed: stats.failed_extraction_tasks,
            retryable_replay_ranges: stats.retryable_extraction_replay_ranges,
            active_replay_ranges: stats.active_extraction_replay_ranges,
            quarantined_replay_ranges: stats.quarantined_extraction_replay_ranges,
            pending_candidates: stats.pending_memory_candidates,
            pending_graph_candidates: stats.pending_graph_candidates,
            oldest_task_epoch: stats.oldest_pending_extraction_epoch,
            oldest_task_age_secs: stats
                .oldest_pending_extraction_epoch
                .map(|epoch| now.saturating_sub(epoch)),
        },
        promotion_funnel: PromotionFunnelStatus {
            captured_events: stats.captured_events,
            observations: stats.total_observations,
            observation_rate_percent: percent(stats.total_observations, stats.captured_events),
            candidates: stats.total_memory_candidates,
            candidate_rate_percent: percent(
                stats.total_memory_candidates,
                stats.total_observations,
            ),
            promoted: stats.promoted_memory_candidates,
            promoted_rate_percent: percent(
                stats.promoted_memory_candidates,
                stats.total_memory_candidates,
            ),
            pending_review: stats.pending_review_memory_candidates,
            pending_review_rate_percent: percent(
                stats.pending_review_memory_candidates,
                stats.total_memory_candidates,
            ),
        },
        usage_feedback: UsageFeedbackStatus {
            citation_events: usage_feedback.total_events,
            citation_line_present_events: usage_feedback.parsed_events,
            citation_line_present_rate_percent: percent(
                usage_feedback.parsed_events,
                usage_feedback.total_events,
            ),
            matched_events: usage_feedback.matched_events,
            match_rate_percent: percent(
                usage_feedback.matched_events,
                usage_feedback.parsed_events,
            ),
            inserted_events: usage_feedback.inserted_events,
            no_citation_events: usage_feedback.no_citation_events,
            unmatched_events: usage_feedback.unmatched_events,
            usage_events: usage_feedback.usage_events,
        },
        pending_observations: PendingObservationStatus {
            ready: stats.ready_pending_observations,
            delayed: stats.delayed_pending_observations,
            processing: stats.processing_pending_observations,
            expired: stats.expired_processing_pending_observations,
            failed: stats.failed_pending_observations,
            oldest_ready_epoch: stats.oldest_ready_pending_epoch,
            oldest_ready_age_secs: stats
                .oldest_ready_pending_epoch
                .map(|epoch| now.saturating_sub(epoch)),
        },
        jobs: JobStatus {
            pending: stats.pending_jobs,
            processing: stats.processing_jobs,
            failed: stats.failed_jobs,
            stuck: stats.stuck_jobs,
        },
        failure_lifecycle: stats.failure_lifecycle,
        worker_daemon: WorkerDaemonStatus {
            health: worker_health_tag(stats.worker_daemon_healthy, stats.worker_heartbeat_age_secs)
                .to_string(),
            heartbeat_age_secs: stats.worker_heartbeat_age_secs,
            owner: stats.worker_heartbeat_owner,
        },
        latest_session_memory_spend: latest_session_memory_spend.map(|spend| {
            LatestSessionMemorySpendStatus {
                session_id: spend.session_id,
                project: spend.project,
                latest_context_epoch: spend.latest_context_epoch,
                context_rows: spend.context_rows,
                context_output_chars: spend.context_output_chars,
                context_estimated_tokens: spend.context_estimated_tokens,
                context_emit_count: spend.context_emit_count,
                context_suppress_count: spend.context_suppress_count,
                ai_usage_attribution: spend.ai_usage_attribution,
                ai_calls: spend.ai_calls,
                ai_total_tokens: spend.ai_total_tokens,
                ai_estimated_cost_usd: spend.ai_estimated_cost_usd,
                ai_unattributed_legacy_calls: spend.ai_unattributed_legacy_calls,
            }
        }),
        candidate_promotion: candidate_promotion
            .into_iter()
            .map(|stat| CandidatePromotionStatus {
                source_kind: stat.source_kind,
                review_status: stat.review_status,
                block_reason: stat.block_reason,
                total: stat.total,
                last_7_days: stat.last_7_days,
            })
            .collect(),
        today: DailyStatus {
            new_memories: daily_stats.memories,
            new_observations: daily_stats.observations,
        },
        top_projects: top_projects
            .into_iter()
            .map(|project| TopProjectStatus {
                project: project.project,
                count: project.count,
            })
            .collect(),
    })
}

fn ensure_status_database_can_migrate(db_path: &std::path::Path) -> Result<()> {
    if !db_path.exists() {
        anyhow::bail!("database not found: {}", db_path.display());
    }

    let conn = db::open_db_read_only()
        .with_context(|| format!("inspect existing remem database {}", db_path.display()))?;
    let has_migration_table: i64 = conn
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master
                WHERE type = 'table' AND name = '_schema_migrations'
             )",
            [],
            |row| row.get(0),
        )
        .with_context(|| format!("inspect remem schema markers in {}", db_path.display()))?;
    let remem_migration_rows = if has_migration_table == 0 {
        0
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM _schema_migrations
             WHERE version = 1 AND name = 'baseline'",
            [],
            |row| row.get(0),
        )
        .with_context(|| format!("inspect remem migration state in {}", db_path.display()))?
    };
    let legacy_core_tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name IN ('memories', 'events')",
            [],
            |row| row.get(0),
        )
        .with_context(|| {
            format!(
                "inspect legacy remem schema markers in {}",
                db_path.display()
            )
        })?;
    let legacy_user_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .with_context(|| format!("inspect legacy remem user_version in {}", db_path.display()))?;

    if remem_migration_rows == 0 && !(legacy_user_version > 0 && legacy_core_tables >= 2) {
        anyhow::bail!(
            "database is not an initialized remem database: {}",
            db_path.display()
        );
    }

    Ok(())
}

fn print_status_report(report: &StatusReport) {
    println!("remem v{}", report.version);
    println!(
        "Database: {} ({:.1} MB)",
        report.database.path, report.database.size_mb
    );
    println!();
    println!("  Memories:      {:>6}", report.totals.memories);
    println!("  Observations:  {:>6}", report.totals.observations);
    println!("  Sessions:      {:>6}", report.totals.sessions);
    println!("  Raw messages:  {:>6}", report.totals.raw_messages);
    println!();
    println!("Embedding:");
    println!(
        "  Provider:     {} -> {}",
        report.embedding.configured_provider, report.embedding.active_provider
    );
    if let Some(fallback) = &report.embedding.fallback_provider {
        println!("  Fallback:     {}", fallback);
    }
    println!(
        "  Model:        {}",
        report
            .embedding
            .active_model_id
            .as_deref()
            .unwrap_or("none")
    );
    println!(
        "  State:        degraded={} disabled={}",
        report.embedding.degraded, report.embedding.disabled
    );
    if let Some(reason) = &report.embedding.unavailable_reason {
        println!("  Unavailable:  {}", reason);
    } else if let Some(reason) = &report.embedding.degradation_reason {
        println!("  Degraded:     {}", reason);
    }
    println!(
        "  Coverage:     {}/{} ({:.1}%)",
        report.embedding.coverage.embedded,
        report.embedding.coverage.total,
        report.embedding.coverage.percent
    );
    if report.embedding.coverage.mixed_profile_count > 1 {
        println!(
            "  Profiles:     {} mixed model/dimension profiles",
            report.embedding.coverage.mixed_profile_count
        );
    }
    println!();
    println!("Raw archive:");
    println!("  Messages:     {:>6}", report.raw_archive.messages);
    println!("  Failures:     {:>6}", report.raw_archive.ingest_failures);
    if report.raw_archive.ingest_failures > 0 {
        println!("  Parse errors: {:>6}", report.raw_archive.parse_errors);
        println!("  Insert err:   {:>6}", report.raw_archive.insert_errors);
        if let Some(kind) = &report.raw_archive.latest_failure_kind {
            println!("  Latest kind:  {}", kind);
        }
        if let Some(path) = &report.raw_archive.latest_failure_path {
            println!("  Latest path:  {}", path);
        }
        if let Some(age_secs) = report.raw_archive.latest_failure_age_secs {
            println!("  Latest age:   {:>6}s", age_secs);
        }
    }
    println!();
    println!("Capture pipeline:");
    println!("  Captured:     {:>6}", report.capture_pipeline.captured);
    println!("  Dropped:      {:>6}", report.capture_pipeline.dropped);
    if report.capture_pipeline.dropped > 0 {
        println!(
            "  Spill open:   {:>6}",
            report.capture_pipeline.unrecovered_spills
        );
        if let Some(reason) = &report.capture_pipeline.latest_drop_reason {
            println!("  Latest drop:  {}", reason);
        }
        if let Some(age_secs) = report.capture_pipeline.latest_drop_age_secs {
            println!("  Drop age:     {:>6}s", age_secs);
        }
    }
    println!(
        "  Extract todo: {:>6}",
        report.capture_pipeline.extract_todo
    );
    println!(
        "  Extract run:  {:>6}",
        report.capture_pipeline.extract_running
    );
    println!(
        "  Extract exp:  {:>6}",
        report.capture_pipeline.extract_expired
    );
    println!(
        "  Extract fail: {:>6}",
        report.capture_pipeline.extract_failed
    );
    println!(
        "  Replay todo:  {:>6}",
        report.capture_pipeline.retryable_replay_ranges
    );
    println!(
        "  Replay run:   {:>6}",
        report.capture_pipeline.active_replay_ranges
    );
    println!(
        "  Replay quar:  {:>6}",
        report.capture_pipeline.quarantined_replay_ranges
    );
    println!(
        "  Candidates:   {:>6}",
        report.capture_pipeline.pending_candidates
    );
    println!(
        "  Graph queue:  {:>6}",
        report.capture_pipeline.pending_graph_candidates
    );
    if let Some(age_secs) = report.capture_pipeline.oldest_task_age_secs {
        println!("  Oldest task:  {:>6}s", age_secs);
    }
    println!();
    println!("Promotion funnel:");
    println!(
        "  Events -> obs: {:>6}/{:<6} ({:>5.1}%)",
        report.promotion_funnel.observations,
        report.promotion_funnel.captured_events,
        report.promotion_funnel.observation_rate_percent
    );
    println!(
        "  Obs -> cand:   {:>6}/{:<6} ({:>5.1}%)",
        report.promotion_funnel.candidates,
        report.promotion_funnel.observations,
        report.promotion_funnel.candidate_rate_percent
    );
    println!(
        "  Cand promoted:{:>6}/{:<6} ({:>5.1}%)",
        report.promotion_funnel.promoted,
        report.promotion_funnel.candidates,
        report.promotion_funnel.promoted_rate_percent
    );
    println!(
        "  Cand pending: {:>6}/{:<6} ({:>5.1}%)",
        report.promotion_funnel.pending_review,
        report.promotion_funnel.candidates,
        report.promotion_funnel.pending_review_rate_percent
    );
    println!();
    println!("Usage feedback:");
    println!(
        "  Citations:   {:>6} events, {:>6} with citation line ({:>5.1}%)",
        report.usage_feedback.citation_events,
        report.usage_feedback.citation_line_present_events,
        report.usage_feedback.citation_line_present_rate_percent
    );
    println!(
        "  Matched:     {:>6}/{:<6} ({:>5.1}% of citation-line events)",
        report.usage_feedback.matched_events,
        report.usage_feedback.citation_line_present_events,
        report.usage_feedback.match_rate_percent
    );
    println!(
        "  No citation: {:>6}",
        report.usage_feedback.no_citation_events
    );
    println!(
        "  Unmatched:   {:>6}",
        report.usage_feedback.unmatched_events
    );
    println!("  Usage rows:  {:>6}", report.usage_feedback.usage_events);
    println!();
    println!("Pending observations:");
    println!("  Ready:        {:>6}", report.pending_observations.ready);
    println!("  Delayed:      {:>6}", report.pending_observations.delayed);
    println!(
        "  Processing:   {:>6}",
        report.pending_observations.processing
    );
    println!("  Expired:      {:>6}", report.pending_observations.expired);
    println!("  Failed:       {:>6}", report.pending_observations.failed);
    if let Some(age_secs) = report.pending_observations.oldest_ready_age_secs {
        println!("  Oldest ready: {:>6}s", age_secs);
    }
    if !report.candidate_promotion.is_empty() {
        println!();
        println!("Candidate promotion:");
        for stat in &report.candidate_promotion {
            let label = match &stat.block_reason {
                Some(reason) => {
                    format!("{} / {} / {}", stat.source_kind, stat.review_status, reason)
                }
                None => format!("{} / {}", stat.source_kind, stat.review_status),
            };
            println!(
                "  {:<48} {:>6}  (7d: {})",
                label, stat.total, stat.last_7_days
            );
        }
    }
    println!();
    println!("Jobs:");
    println!("  Pending:      {:>6}", report.jobs.pending);
    println!("  Processing:   {:>6}", report.jobs.processing);
    println!("  Failed:       {:>6}", report.jobs.failed);
    println!("  Stuck:        {:>6}", report.jobs.stuck);
    println!();
    println!("Failures:");
    print_failure_surface("Pending obs", &report.failure_lifecycle.pending_observation);
    print_failure_surface("Extraction", &report.failure_lifecycle.extraction_task);
    print_failure_surface(
        "Replay rng",
        &report.failure_lifecycle.extraction_replay_range,
    );
    print_failure_surface("Jobs", &report.failure_lifecycle.job);
    println!();
    println!("Worker daemon:");
    println!("  Health:       {:>7}", report.worker_daemon.health);
    if let Some(age_secs) = report.worker_daemon.heartbeat_age_secs {
        println!("  Last beat:    {:>6}s", age_secs);
    }
    if let Some(owner) = &report.worker_daemon.owner {
        println!("  Owner:        {}", owner);
    }
    if report.worker_daemon.health == "missing" || report.worker_daemon.health == "stale" {
        println!("  Fallback:     {}", worker_once_fallback_human());
    }
    println!();
    println!("Today:");
    println!("  New memories:      {:>4}", report.today.new_memories);
    println!("  New observations:  {:>4}", report.today.new_observations);

    if let Some(spend) = &report.latest_session_memory_spend {
        println!();
        println!("Latest session memory footprint:");
        println!("  Session:      {}", spend.session_id);
        println!("  Project:      {}", spend.project);
        println!(
            "  Context now:  {:>6} chars (~{} tokens)",
            spend.context_output_chars, spend.context_estimated_tokens
        );
        println!(
            "  Context runs: {:>6} emitted, {:>6} suppressed",
            spend.context_emit_count, spend.context_suppress_count
        );
        match spend.ai_usage_attribution.as_str() {
            "attributed" => {
                println!(
                    "  AI usage:     {:>6} calls, {:>6} tokens, ${:.4}",
                    spend.ai_calls, spend.ai_total_tokens, spend.ai_estimated_cost_usd
                );
            }
            "partial" => {
                println!(
                    "  AI usage:     {:>6} attributed calls, {:>6} tokens, ${:.4}",
                    spend.ai_calls, spend.ai_total_tokens, spend.ai_estimated_cost_usd
                );
                println!(
                    "  AI legacy:    {:>6} unattributed calls not assigned to a session",
                    spend.ai_unattributed_legacy_calls
                );
            }
            _ => {
                println!("  AI usage:     unavailable on legacy rows without session_id");
            }
        }
    }

    if !report.top_projects.is_empty() {
        println!();
        println!("Top projects:");
        for project in &report.top_projects {
            println!("  {:>4}  {}", project.count, project.project);
        }
    }

    let actions = status_health_actions(report);
    let action_block = render_action_block(&actions);
    if !action_block.is_empty() {
        println!();
        print!("{action_block}");
    }
}

fn print_failure_surface(label: &str, stats: &db::FailureSurfaceStats) {
    println!(
        "  {:<11} actionable={:>5} 7d={:>5} transient={:>5} permanent={:>5} archived={:>5}",
        label,
        stats.actionable_total,
        stats.actionable_7d,
        stats.transient,
        stats.permanent,
        stats.archived
    );
}

fn worker_health_tag(healthy: bool, heartbeat_age_secs: Option<i64>) -> &'static str {
    if healthy {
        "healthy"
    } else if heartbeat_age_secs.is_some() {
        "stale"
    } else {
        "missing"
    }
}

fn percent(numerator: i64, denominator: i64) -> f64 {
    if denominator <= 0 {
        0.0
    } else {
        (numerator as f64 * 100.0) / denominator as f64
    }
}

fn status_health_actions(report: &StatusReport) -> Vec<crate::doctor::health_action::HealthAction> {
    queue_actions_with_replay(
        report.pending_observations.failed,
        report.pending_observations.expired,
        report.capture_pipeline.extract_expired,
        report.jobs.failed,
        report.jobs.stuck,
        report.capture_pipeline.extract_failed,
        report.capture_pipeline.retryable_replay_ranges,
    )
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct StatusReport {
    pub version: String,
    pub database: StatusDatabase,
    pub totals: StatusTotals,
    pub embedding: EmbeddingStatus,
    pub raw_archive: RawArchiveStatus,
    pub capture_pipeline: CapturePipelineStatus,
    pub promotion_funnel: PromotionFunnelStatus,
    pub usage_feedback: UsageFeedbackStatus,
    pub pending_observations: PendingObservationStatus,
    pub candidate_promotion: Vec<CandidatePromotionStatus>,
    pub jobs: JobStatus,
    pub failure_lifecycle: db::FailureLifecycleStats,
    pub worker_daemon: WorkerDaemonStatus,
    pub latest_session_memory_spend: Option<LatestSessionMemorySpendStatus>,
    pub today: DailyStatus,
    pub top_projects: Vec<TopProjectStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct StatusDatabase {
    pub path: String,
    pub size_bytes: u64,
    pub size_mb: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct StatusTotals {
    pub memories: i64,
    pub observations: i64,
    pub sessions: i64,
    pub raw_messages: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct EmbeddingStatus {
    pub configured_provider: String,
    pub fallback_provider: Option<String>,
    pub active_provider: String,
    pub active_model_id: Option<String>,
    pub degraded: bool,
    pub disabled: bool,
    pub unavailable_reason: Option<String>,
    pub degradation_reason: Option<String>,
    pub coverage: EmbeddingCoverageStatus,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct EmbeddingCoverageStatus {
    pub embedded: i64,
    pub total: i64,
    pub percent: f64,
    pub mixed_profile_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct RawArchiveStatus {
    pub messages: i64,
    pub ingest_failures: i64,
    pub parse_errors: i64,
    pub insert_errors: i64,
    pub latest_failure_epoch: Option<i64>,
    pub latest_failure_age_secs: Option<i64>,
    pub latest_failure_kind: Option<String>,
    pub latest_failure_path: Option<String>,
    pub latest_failure_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CapturePipelineStatus {
    pub captured: i64,
    pub dropped: i64,
    pub unrecovered_spills: i64,
    pub latest_drop_epoch: Option<i64>,
    pub latest_drop_age_secs: Option<i64>,
    pub latest_drop_reason: Option<String>,
    pub latest_drop_detail: Option<String>,
    pub extract_todo: i64,
    pub extract_running: i64,
    pub extract_expired: i64,
    pub extract_failed: i64,
    pub retryable_replay_ranges: i64,
    pub active_replay_ranges: i64,
    pub quarantined_replay_ranges: i64,
    pub pending_candidates: i64,
    pub pending_graph_candidates: i64,
    pub oldest_task_epoch: Option<i64>,
    pub oldest_task_age_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct PromotionFunnelStatus {
    pub captured_events: i64,
    pub observations: i64,
    pub observation_rate_percent: f64,
    pub candidates: i64,
    pub candidate_rate_percent: f64,
    pub promoted: i64,
    pub promoted_rate_percent: f64,
    pub pending_review: i64,
    pub pending_review_rate_percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct UsageFeedbackStatus {
    pub citation_events: i64,
    pub citation_line_present_events: i64,
    pub citation_line_present_rate_percent: f64,
    pub matched_events: i64,
    pub match_rate_percent: f64,
    pub inserted_events: i64,
    pub no_citation_events: i64,
    pub unmatched_events: i64,
    pub usage_events: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct PendingObservationStatus {
    pub ready: i64,
    pub delayed: i64,
    pub processing: i64,
    pub expired: i64,
    pub failed: i64,
    pub oldest_ready_epoch: Option<i64>,
    pub oldest_ready_age_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CandidatePromotionStatus {
    pub source_kind: String,
    pub review_status: String,
    pub block_reason: Option<String>,
    pub total: i64,
    pub last_7_days: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct JobStatus {
    pub pending: i64,
    pub processing: i64,
    pub failed: i64,
    pub stuck: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct WorkerDaemonStatus {
    pub health: String,
    pub heartbeat_age_secs: Option<i64>,
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct LatestSessionMemorySpendStatus {
    pub session_id: String,
    pub project: String,
    pub latest_context_epoch: i64,
    pub context_rows: i64,
    pub context_output_chars: i64,
    pub context_estimated_tokens: i64,
    pub context_emit_count: i64,
    pub context_suppress_count: i64,
    pub ai_usage_attribution: String,
    pub ai_calls: i64,
    pub ai_total_tokens: i64,
    pub ai_estimated_cost_usd: f64,
    pub ai_unattributed_legacy_calls: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct DailyStatus {
    pub new_memories: i64,
    pub new_observations: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct TopProjectStatus {
    pub project: String,
    pub count: i64,
}

#[cfg(test)]
mod tests;
