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
    let conn = db::open_db_read_only()?;
    let db_path = db::db_path();
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
        worker_daemon: WorkerDaemonStatus {
            health: worker_health_tag(stats.worker_daemon_healthy, stats.worker_heartbeat_age_secs)
                .to_string(),
            heartbeat_age_secs: stats.worker_heartbeat_age_secs,
            owner: stats.worker_heartbeat_owner,
        },
        candidate_promotion: candidate_promotion
            .into_iter()
            .map(|stat| CandidatePromotionStatus {
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
                Some(reason) => format!("{} / {}", stat.review_status, reason),
                None => stat.review_status.clone(),
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

fn worker_health_tag(healthy: bool, heartbeat_age_secs: Option<i64>) -> &'static str {
    if healthy {
        "healthy"
    } else if heartbeat_age_secs.is_some() {
        "stale"
    } else {
        "missing"
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
    pub raw_archive: RawArchiveStatus,
    pub capture_pipeline: CapturePipelineStatus,
    pub pending_observations: PendingObservationStatus,
    pub candidate_promotion: Vec<CandidatePromotionStatus>,
    pub jobs: JobStatus,
    pub worker_daemon: WorkerDaemonStatus,
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
mod tests {
    use serde_json::Value;

    use super::*;

    fn status_report_fixture() -> StatusReport {
        StatusReport {
            version: "0.4.5".to_string(),
            database: StatusDatabase {
                path: "/tmp/remem-test".to_string(),
                size_bytes: 1_048_576,
                size_mb: 1.0,
            },
            totals: StatusTotals {
                memories: 1,
                observations: 2,
                sessions: 3,
                raw_messages: 4,
            },
            raw_archive: RawArchiveStatus {
                messages: 4,
                ingest_failures: 0,
                parse_errors: 0,
                insert_errors: 0,
                latest_failure_epoch: None,
                latest_failure_age_secs: None,
                latest_failure_kind: None,
                latest_failure_path: None,
                latest_failure_message: None,
            },
            capture_pipeline: CapturePipelineStatus {
                captured: 5,
                dropped: 0,
                unrecovered_spills: 0,
                latest_drop_epoch: None,
                latest_drop_age_secs: None,
                latest_drop_reason: None,
                latest_drop_detail: None,
                extract_todo: 6,
                extract_running: 7,
                extract_expired: 0,
                extract_failed: 0,
                retryable_replay_ranges: 0,
                active_replay_ranges: 0,
                quarantined_replay_ranges: 0,
                pending_candidates: 9,
                pending_graph_candidates: 10,
                oldest_task_epoch: Some(10),
                oldest_task_age_secs: Some(11),
            },
            pending_observations: PendingObservationStatus {
                ready: 12,
                delayed: 13,
                processing: 14,
                expired: 0,
                failed: 0,
                oldest_ready_epoch: Some(17),
                oldest_ready_age_secs: Some(18),
            },
            candidate_promotion: vec![CandidatePromotionStatus {
                review_status: "pending_review".to_string(),
                block_reason: Some("no_supporting_source_observation".to_string()),
                total: 41,
                last_7_days: 6,
            }],
            jobs: JobStatus {
                pending: 19,
                processing: 20,
                failed: 0,
                stuck: 0,
            },
            worker_daemon: WorkerDaemonStatus {
                health: "healthy".to_string(),
                heartbeat_age_secs: Some(23),
                owner: Some("worker-1".to_string()),
            },
            today: DailyStatus {
                new_memories: 24,
                new_observations: 25,
            },
            top_projects: vec![TopProjectStatus {
                project: "proj".to_string(),
                count: 26,
            }],
        }
    }

    #[test]
    fn cli_status_json_report_is_machine_parseable() -> std::result::Result<(), serde_json::Error> {
        let mut report = status_report_fixture();
        report.raw_archive.ingest_failures = 1;
        report.raw_archive.parse_errors = 2;
        report.raw_archive.insert_errors = 3;
        report.raw_archive.latest_failure_kind = Some("mixed_errors".to_string());
        report.raw_archive.latest_failure_path = Some("/bad/raw.jsonl".to_string());
        report.pending_observations.expired = 15;
        report.pending_observations.failed = 16;
        report.jobs.failed = 21;
        report.jobs.stuck = 22;

        let text = serde_json::to_string(&report)?;
        let parsed: Value = serde_json::from_str(&text)?;

        assert_eq!(parsed["version"], "0.4.5");
        assert_eq!(parsed["database"]["size_bytes"], 1_048_576);
        assert_eq!(parsed["totals"]["memories"], 1);
        assert_eq!(parsed["raw_archive"]["messages"], 4);
        assert_eq!(parsed["raw_archive"]["ingest_failures"], 1);
        assert_eq!(parsed["raw_archive"]["parse_errors"], 2);
        assert_eq!(parsed["raw_archive"]["insert_errors"], 3);
        assert_eq!(parsed["raw_archive"]["latest_failure_kind"], "mixed_errors");
        assert_eq!(
            parsed["raw_archive"]["latest_failure_path"],
            "/bad/raw.jsonl"
        );
        assert_eq!(parsed["capture_pipeline"]["extract_todo"], 6);
        assert_eq!(parsed["capture_pipeline"]["pending_graph_candidates"], 10);
        assert_eq!(parsed["pending_observations"]["failed"], 16);
        assert_eq!(
            parsed["candidate_promotion"][0]["review_status"],
            "pending_review"
        );
        assert_eq!(
            parsed["candidate_promotion"][0]["block_reason"],
            "no_supporting_source_observation"
        );
        assert_eq!(parsed["candidate_promotion"][0]["total"], 41);
        assert_eq!(parsed["candidate_promotion"][0]["last_7_days"], 6);
        assert_eq!(parsed["worker_daemon"]["health"], "healthy");
        assert_eq!(parsed["top_projects"][0]["project"], "proj");
        Ok(())
    }

    #[test]
    fn cli_status_has_no_action_block_when_runtime_is_clear() {
        let report = status_report_fixture();
        let actions = status_health_actions(&report);

        assert!(render_action_block(&actions).is_empty());
    }

    #[test]
    fn cli_status_renders_action_block_for_runtime_failures() {
        let mut report = status_report_fixture();
        report.pending_observations.failed = 43;
        report.pending_observations.expired = 1;
        report.capture_pipeline.extract_failed = 4;
        report.jobs.failed = 2;
        report.jobs.stuck = 3;

        let actions = status_health_actions(&report);
        let text = render_action_block(&actions);

        assert!(text.contains("Needs attention:"));
        assert!(text.contains("43 failed pending observations"));
        assert!(text.contains("inspect: remem pending list-failed --limit 20"));
        assert!(text.contains("preview retry: remem pending retry-failed --dry-run"));
        assert!(text.contains("1 expired processing pending observation"));
        assert!(text.contains("4 failed extraction tasks"));
        assert!(text.contains("2 failed jobs"));
        assert!(text.contains("3 stuck jobs"));
        assert!(text.contains("inspect counts: remem status --json"));
        assert!(text.contains("recover: remem worker --once"));
    }
}
