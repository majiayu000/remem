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
        embedding: EmbeddingStatus {
            configured_provider: "local".to_string(),
            fallback_provider: Some("feature-hash".to_string()),
            active_provider: "local".to_string(),
            active_model_id: Some("remem-local-feature-hash-v1".to_string()),
            degraded: false,
            disabled: false,
            unavailable_reason: None,
            degradation_reason: None,
            coverage: EmbeddingCoverageStatus {
                embedded: 8,
                total: 10,
                percent: 80.0,
                mixed_profile_count: 1,
            },
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
        legacy_surfaces: vec![
            LegacySurfaceStatus {
                surface: "pending_observations".to_string(),
                disposition: "retire".to_string(),
                row_count: 2,
                last_write_epoch: Some(120),
                last_write_age_secs: Some(30),
                frozen_write_violations: 2,
            },
            LegacySurfaceStatus {
                surface: "summary_jobs".to_string(),
                disposition: "retire-summary-only".to_string(),
                row_count: 1,
                last_write_epoch: Some(130),
                last_write_age_secs: Some(20),
                frozen_write_violations: 1,
            },
        ],
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
        review_queue: ReviewQueueStatus {
            pending: 41,
            median_age_secs: Some(86_400),
            max_age_secs: Some(172_800),
            inflow_7d: 6,
            resolved_7d: 2,
            projects: vec![ReviewQueueProjectStatus {
                project: Some("/tmp/remem".to_string()),
                pending: 41,
                median_age_secs: Some(86_400),
                max_age_secs: Some(172_800),
                inflow_7d: 6,
                resolved_7d: 2,
            }],
            block_reasons: vec![ReviewQueueBlockReasonStatus {
                reason: Some("risk_class_not_low".to_string()),
                pending: 40,
                example_ids: vec![1, 2, 3],
            }],
        },
        candidate_promotion: vec![CandidatePromotionStatus {
            source_kind: "summary".to_string(),
            review_status: "pending_review".to_string(),
            block_reason: Some("summary_gate_shadow".to_string()),
            total: 41,
            last_7_days: 6,
        }],
        user_context: UserContextStatus {
            claims_total: 5,
            claims_active: 3,
            claims_suppressed: 1,
            claims_deleted: 1,
            candidates_total: 7,
            candidates_pending_review: 4,
            candidates_auto_promoted: 2,
            candidate_block_reasons: vec![UserContextBlockReasonStatus {
                reason: Some("source_not_user_authored".to_string()),
                pending: 3,
            }],
        },
        jobs: JobStatus {
            pending: 19,
            processing: 20,
            failed: 0,
            stuck: 0,
        },
        failure_lifecycle: crate::db::FailureLifecycleStats::default(),
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
    assert_eq!(parsed["embedding"]["configured_provider"], "local");
    assert_eq!(parsed["embedding"]["fallback_provider"], "feature-hash");
    assert_eq!(parsed["embedding"]["active_provider"], "local");
    assert_eq!(
        parsed["embedding"]["active_model_id"],
        "remem-local-feature-hash-v1"
    );
    assert_eq!(parsed["embedding"]["degraded"], false);
    assert_eq!(parsed["embedding"]["disabled"], false);
    assert_eq!(parsed["embedding"]["coverage"]["embedded"], 8);
    assert_eq!(parsed["embedding"]["coverage"]["total"], 10);
    assert_eq!(parsed["embedding"]["coverage"]["mixed_profile_count"], 1);
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
    assert_eq!(
        parsed["legacy_surfaces"][0]["surface"],
        "pending_observations"
    );
    assert_eq!(parsed["legacy_surfaces"][0]["disposition"], "retire");
    assert_eq!(parsed["legacy_surfaces"][0]["row_count"], 2);
    assert_eq!(parsed["legacy_surfaces"][0]["last_write_epoch"], 120);
    assert_eq!(parsed["legacy_surfaces"][0]["last_write_age_secs"], 30);
    assert_eq!(parsed["legacy_surfaces"][0]["frozen_write_violations"], 2);
    assert_eq!(parsed["legacy_surfaces"][1]["surface"], "summary_jobs");
    assert_eq!(parsed["usage_feedback"]["citation_events"], 7);
    assert_eq!(parsed["usage_feedback"]["citation_line_present_events"], 5);
    assert_eq!(parsed["usage_feedback"]["matched_events"], 4);
    assert_eq!(parsed["usage_feedback"]["no_citation_events"], 2);
    assert_eq!(parsed["usage_feedback"]["unmatched_events"], 1);
    assert_eq!(parsed["usage_feedback"]["usage_events"], 6);
    assert_eq!(parsed["pending_observations"]["failed"], 16);
    assert_eq!(parsed["review_queue"]["pending"], 41);
    assert_eq!(parsed["review_queue"]["median_age_secs"], 86_400);
    assert_eq!(parsed["review_queue"]["max_age_secs"], 172_800);
    assert_eq!(parsed["review_queue"]["inflow_7d"], 6);
    assert_eq!(parsed["review_queue"]["resolved_7d"], 2);
    assert_eq!(parsed["review_queue"]["projects"][0]["pending"], 41);
    assert_eq!(
        parsed["review_queue"]["block_reasons"][0]["reason"],
        "risk_class_not_low"
    );
    assert_eq!(
        parsed["review_queue"]["block_reasons"][0]["example_ids"][0],
        1
    );
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
    assert_eq!(parsed["user_context"]["claims_total"], 5);
    assert_eq!(parsed["user_context"]["claims_active"], 3);
    assert_eq!(parsed["user_context"]["claims_suppressed"], 1);
    assert_eq!(parsed["user_context"]["claims_deleted"], 1);
    assert_eq!(parsed["user_context"]["candidates_total"], 7);
    assert_eq!(parsed["user_context"]["candidates_pending_review"], 4);
    assert_eq!(parsed["user_context"]["candidates_auto_promoted"], 2);
    assert_eq!(
        parsed["user_context"]["candidate_block_reasons"][0]["reason"],
        "source_not_user_authored"
    );
    assert_eq!(
        parsed["user_context"]["candidate_block_reasons"][0]["pending"],
        3
    );
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
    let mut report = status_report_fixture();
    report.pending_observations.ready = 0;
    report.pending_observations.delayed = 0;
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
    assert!(text.contains("preview migration prep: remem pending retry-failed --dry-run"));
    assert!(text.contains("apply migration prep: remem pending retry-failed"));
    assert!(text.contains("preview replay: remem pending migrate-legacy --dry-run"));
    assert!(text.contains("apply replay: remem pending migrate-legacy"));
    assert!(text
        .contains("apply replay for Claude host: remem pending migrate-legacy --host claude-code"));
    assert!(
        text.contains("apply replay for Codex host: remem pending migrate-legacy --host codex-cli")
    );
    assert!(text.contains("26 replayable legacy pending observations"));
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
fn status_report_refuses_non_remem_migration_table_without_initializing() -> anyhow::Result<()> {
    let test_dir =
        crate::db::test_support::ScopedTestDataDir::new("status-non-remem-migration-table");
    std::fs::create_dir_all(&test_dir.path)?;
    let conn = Connection::open(test_dir.db_path())?;
    conn.execute_batch(
        "CREATE TABLE _schema_migrations (
             version INTEGER PRIMARY KEY,
             name TEXT NOT NULL,
             applied_at_epoch INTEGER NOT NULL
         );
         INSERT INTO _schema_migrations VALUES (1, 'other_app_baseline', 0);",
    )?;
    drop(conn);

    let err = load_status_report().expect_err("non-remem migration table should fail");

    let message = err.to_string();
    assert!(
        message.contains("not an initialized remem database"),
        "unexpected error: {message}"
    );
    let conn = Connection::open(test_dir.db_path())?;
    let memories_created: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'memories'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(memories_created, 0);
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
