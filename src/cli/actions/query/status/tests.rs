use rusqlite::{params, Connection};
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
        promotion_funnel: PromotionFunnelStatus {
            captured_events: 5,
            observations: 4,
            observation_rate_percent: 80.0,
            candidates: 3,
            candidate_rate_percent: 75.0,
            promoted: 2,
            promoted_rate_percent: 66.66666666666667,
            pending_review: 1,
            pending_review_rate_percent: 33.333333333333336,
        },
        usage_feedback: UsageFeedbackStatus {
            citation_events: 7,
            citation_line_present_events: 5,
            citation_line_present_rate_percent: 71.42857142857143,
            matched_events: 4,
            match_rate_percent: 80.0,
            inserted_events: 4,
            no_citation_events: 2,
            unmatched_events: 1,
            usage_events: 6,
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
            source_kind: "summary".to_string(),
            review_status: "pending_review".to_string(),
            block_reason: Some("summary_gate_shadow".to_string()),
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
        latest_session_memory_spend: Some(LatestSessionMemorySpendStatus {
            session_id: "sess-1".to_string(),
            project: "/tmp/remem".to_string(),
            latest_context_epoch: 1_800_000_000,
            context_rows: 2,
            context_output_chars: 3_201,
            context_estimated_tokens: 801,
            context_emit_count: 3,
            context_suppress_count: 1,
            ai_usage_attribution: "partial".to_string(),
            ai_calls: 2,
            ai_total_tokens: 1_234,
            ai_estimated_cost_usd: 0.0123,
            ai_unattributed_legacy_calls: 1,
        }),
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
    assert_eq!(parsed["promotion_funnel"]["captured_events"], 5);
    assert_eq!(parsed["promotion_funnel"]["observations"], 4);
    assert_eq!(parsed["promotion_funnel"]["candidates"], 3);
    assert_eq!(parsed["promotion_funnel"]["promoted"], 2);
    assert_eq!(parsed["promotion_funnel"]["pending_review"], 1);
    assert_eq!(parsed["usage_feedback"]["citation_events"], 7);
    assert_eq!(parsed["usage_feedback"]["citation_line_present_events"], 5);
    assert_eq!(parsed["usage_feedback"]["matched_events"], 4);
    assert_eq!(parsed["usage_feedback"]["no_citation_events"], 2);
    assert_eq!(parsed["usage_feedback"]["unmatched_events"], 1);
    assert_eq!(parsed["usage_feedback"]["usage_events"], 6);
    assert_eq!(parsed["pending_observations"]["failed"], 16);
    assert_eq!(parsed["candidate_promotion"][0]["source_kind"], "summary");
    assert_eq!(
        parsed["candidate_promotion"][0]["review_status"],
        "pending_review"
    );
    assert_eq!(
        parsed["candidate_promotion"][0]["block_reason"],
        "summary_gate_shadow"
    );
    assert_eq!(parsed["candidate_promotion"][0]["total"], 41);
    assert_eq!(parsed["candidate_promotion"][0]["last_7_days"], 6);
    assert_eq!(parsed["worker_daemon"]["health"], "healthy");
    assert_eq!(
        parsed["latest_session_memory_spend"]["session_id"],
        "sess-1"
    );
    assert_eq!(
        parsed["latest_session_memory_spend"]["context_estimated_tokens"],
        801
    );
    assert_eq!(
        parsed["latest_session_memory_spend"]["ai_usage_attribution"],
        "partial"
    );
    assert_eq!(
        parsed["latest_session_memory_spend"]["ai_unattributed_legacy_calls"],
        1
    );
    assert_eq!(
        parsed["latest_session_memory_spend"]["ai_total_tokens"],
        1234
    );
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

#[test]
fn status_report_refuses_missing_database_without_initializing() {
    let test_dir = crate::db::test_support::ScopedTestDataDir::new("status-missing-db");

    let err = load_status_report().expect_err("missing database should fail");

    let message = err.to_string();
    assert!(
        message.contains("database not found"),
        "unexpected error: {message}"
    );
    assert!(
        !test_dir.path.exists(),
        "status must not create data dir for a missing database"
    );
    assert!(
        !test_dir.db_path().exists(),
        "status must not initialize a missing database"
    );
}

#[test]
fn status_report_refuses_empty_database_file_without_initializing() -> anyhow::Result<()> {
    let test_dir = crate::db::test_support::ScopedTestDataDir::new("status-empty-db");
    std::fs::create_dir_all(&test_dir.path)?;
    std::fs::write(test_dir.db_path(), [])?;

    let err = load_status_report().expect_err("empty database file should fail");

    let message = err.to_string();
    assert!(
        message.contains("not an initialized remem database"),
        "unexpected error: {message}"
    );
    assert_eq!(
        std::fs::metadata(test_dir.db_path())?.len(),
        0,
        "status must not initialize an empty existing database file"
    );
    Ok(())
}

#[test]
fn status_report_migrates_v053_candidate_source_kind_schema() -> anyhow::Result<()> {
    let _test_dir = crate::db::test_support::ScopedTestDataDir::new("status-v053-source-kind");
    std::fs::create_dir_all(crate::db::data_dir())?;
    let conn = Connection::open(crate::db::db_path())?;
    conn.execute_batch(
        "PRAGMA foreign_keys=OFF;
         CREATE TABLE IF NOT EXISTS _schema_migrations (
             version INTEGER PRIMARY KEY,
             name TEXT NOT NULL,
             applied_at_epoch INTEGER NOT NULL
         );",
    )?;
    for migration in crate::migrate::MIGRATIONS
        .iter()
        .filter(|migration| migration.version <= 53)
    {
        conn.execute_batch(migration.sql)?;
        conn.execute(
            "INSERT OR IGNORE INTO _schema_migrations (version, name, applied_at_epoch)
             VALUES (?1, ?2, 0)",
            params![migration.version, migration.name],
        )?;
    }
    conn.execute_batch("PRAGMA user_version = 65")?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO memory_candidates(project_id, scope, memory_type, topic_key, text,
                                       evidence_event_ids, confidence, risk_class,
                                       review_status, auto_promote_block_reason,
                                       created_at_epoch, updated_at_epoch)
         VALUES (1, 'project', 'decision', 'summary-gate', 'summary fact',
                 '[1]', 0.9, 'low', 'pending_review', 'summary_gate_shadow',
                 ?1, ?1)",
        params![now],
    )?;
    drop(conn);

    let report = load_status_report()?;

    assert_eq!(report.candidate_promotion.len(), 1);
    let stat = &report.candidate_promotion[0];
    assert_eq!(stat.source_kind, "unattributed");
    assert_eq!(stat.review_status, "pending_review");
    assert_eq!(stat.block_reason.as_deref(), Some("summary_gate_shadow"));
    assert_eq!(stat.total, 1);

    let conn = Connection::open(crate::db::db_path())?;
    let source_kind: String =
        conn.query_row("SELECT source_kind FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    let v054_applied: i64 = conn.query_row(
        "SELECT COUNT(*) FROM _schema_migrations WHERE version = 54",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(source_kind, "unattributed");
    assert_eq!(v054_applied, 1);
    Ok(())
}
