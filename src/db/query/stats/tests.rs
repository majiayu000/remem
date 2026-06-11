use rusqlite::Connection;

use super::{CandidatePromotionStat, DailyActivityStats, ProjectCount, SystemStats};
use crate::db::models::{
    AiUsageBreakdown, AiUsageSourceTotals, AiUsageTotals, DailyAiUsage, WeeklyAiUsage,
};
use crate::db::query::{
    query_ai_usage_breakdown, query_ai_usage_source_totals, query_ai_usage_totals,
    query_candidate_promotion_stats, query_daily_activity_stats, query_daily_ai_usage,
    query_memory_facts_stats, query_system_stats, query_top_projects, query_weekly_ai_usage,
};

fn setup_stats_schema(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE memories (
            id INTEGER PRIMARY KEY,
            project TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL,
            expires_at_epoch INTEGER
        );
        CREATE TABLE observations (
            id INTEGER PRIMARY KEY,
            project TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL
        );
        CREATE TABLE session_summaries (
            id INTEGER PRIMARY KEY
        );
        CREATE TABLE raw_messages (
            id INTEGER PRIMARY KEY
        );
        CREATE TABLE raw_ingest_failures (
            id INTEGER PRIMARY KEY,
            transcript_path TEXT,
            error_kind TEXT NOT NULL,
            error_message TEXT NOT NULL,
            parse_errors INTEGER NOT NULL,
            insert_errors INTEGER NOT NULL,
            created_at_epoch INTEGER NOT NULL
        );
        CREATE TABLE captured_events (
            id INTEGER PRIMARY KEY
        );
        CREATE TABLE memory_facts (
            id INTEGER PRIMARY KEY,
            status TEXT NOT NULL,
            valid_from_epoch INTEGER,
            source_memory_id INTEGER
        );
        CREATE TABLE capture_drop_events (
            id INTEGER PRIMARY KEY,
            host_id TEXT,
            session_id TEXT,
            project TEXT,
            tool_name TEXT,
            reason TEXT NOT NULL,
            detail TEXT,
            spill_path TEXT,
            recovered_event_id INTEGER,
            created_at_epoch INTEGER NOT NULL,
            recovered_at_epoch INTEGER
        );
        CREATE TABLE extraction_tasks (
            id INTEGER PRIMARY KEY,
            status TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL,
            lease_expires_epoch INTEGER
        );
        CREATE TABLE memory_candidates (
            id INTEGER PRIMARY KEY,
            review_status TEXT NOT NULL,
            auto_promote_block_reason TEXT,
            created_at_epoch INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE graph_candidates (
            id INTEGER PRIMARY KEY,
            review_status TEXT NOT NULL
        );
        CREATE TABLE pending_observations (
            id INTEGER PRIMARY KEY,
            status TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL DEFAULT 0,
            next_retry_epoch INTEGER,
            lease_owner TEXT,
            lease_expires_epoch INTEGER
        );
        CREATE TABLE jobs (
            id INTEGER PRIMARY KEY,
            state TEXT NOT NULL,
            lease_expires_epoch INTEGER
        );
        CREATE TABLE worker_heartbeats (
            owner TEXT PRIMARY KEY,
            pid INTEGER,
            started_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL
        );
        CREATE TABLE ai_usage_events (
            id INTEGER PRIMARY KEY,
            created_at TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL,
            project TEXT,
            operation TEXT NOT NULL,
            executor TEXT NOT NULL,
            model TEXT,
            input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            raw_input_tokens INTEGER NOT NULL DEFAULT 0,
            raw_output_tokens INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL,
            estimated_cost_usd REAL NOT NULL,
            usage_source TEXT NOT NULL DEFAULT 'text_estimate',
            pricing_source TEXT NOT NULL DEFAULT 'remem_static'
        );",
    )
    .expect("schema should be created");
}

#[test]
fn query_system_stats_and_related_views_share_one_definition() {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    setup_stats_schema(&conn);

    conn.execute(
        "INSERT INTO memories (project, status, created_at_epoch) VALUES ('alpha', 'active', 200)",
        [],
    )
    .expect("active memory insert should succeed");
    conn.execute(
        "INSERT INTO memories (project, status, created_at_epoch) VALUES ('alpha', 'archived', 150)",
        [],
    )
    .expect("archived memory insert should succeed");
    conn.execute(
        "INSERT INTO memories (project, status, created_at_epoch) VALUES ('beta', 'active', 300)",
        [],
    )
    .expect("second active memory insert should succeed");
    if let Err(err) = conn.execute(
        "INSERT INTO memories (project, status, created_at_epoch, expires_at_epoch)
         VALUES ('gamma', 'active', 310, strftime('%s', 'now') - 1)",
        [],
    ) {
        panic!("expired active memory insert should succeed: {err}");
    }
    if let Err(err) = conn.execute(
        "INSERT INTO memories (project, status, created_at_epoch, expires_at_epoch)
         VALUES ('delta', 'active', 320, CAST(strftime('%s', 'now') AS INTEGER) + 3600)",
        [],
    ) {
        panic!("future-expiring active memory insert should succeed: {err}");
    }
    conn.execute(
        "INSERT INTO observations (project, status, created_at_epoch) VALUES ('alpha', 'active', 220)",
        [],
    )
    .expect("active observation insert should succeed");
    conn.execute(
        "INSERT INTO observations (project, status, created_at_epoch) VALUES ('beta', 'stale', 140)",
        [],
    )
    .expect("stale observation insert should succeed");
    conn.execute("INSERT INTO session_summaries (id) VALUES (1)", [])
        .expect("summary insert should succeed");
    conn.execute(
        "INSERT INTO raw_ingest_failures
         (transcript_path, error_kind, error_message, parse_errors, insert_errors, created_at_epoch)
         VALUES ('/bad/transcript.jsonl', 'parse_errors', 'bad jsonl', 2, 1, 160)",
        [],
    )
    .expect("raw ingest failure insert should succeed");
    conn.execute("INSERT INTO captured_events (id) VALUES (1)", [])
        .expect("captured event insert should succeed");
    conn.execute(
        "INSERT INTO capture_drop_events
         (host_id, session_id, project, tool_name, reason, detail, created_at_epoch)
         VALUES ('codex-cli', 'session-drop', 'alpha', 'Bash', 'codex_bash_disabled',
                 'Codex Bash capture disabled', 170)",
        [],
    )
    .expect("capture drop insert should succeed");
    conn.execute(
        "INSERT INTO extraction_tasks (status, created_at_epoch) VALUES ('pending', 90)",
        [],
    )
    .expect("pending extraction task insert should succeed");
    conn.execute(
        "INSERT INTO extraction_tasks (status, created_at_epoch, lease_expires_epoch)
         VALUES ('processing', 95, strftime('%s', 'now') - 1)",
        [],
    )
    .expect("processing extraction task insert should succeed");
    conn.execute(
        "INSERT INTO extraction_tasks (status, created_at_epoch) VALUES ('failed', 96)",
        [],
    )
    .expect("failed extraction task insert should succeed");
    conn.execute(
        "INSERT INTO memory_candidates (review_status) VALUES ('pending_review')",
        [],
    )
    .expect("memory candidate insert should succeed");
    conn.execute(
        "INSERT INTO graph_candidates (review_status) VALUES ('pending_review')",
        [],
    )
    .expect("graph candidate insert should succeed");
    if let Err(err) = conn.execute(
        "INSERT INTO graph_candidates (review_status) VALUES ('deferred')",
        [],
    ) {
        panic!("deferred graph candidate insert should succeed: {err}");
    }
    conn.execute(
        "INSERT INTO graph_candidates (review_status) VALUES ('approved')",
        [],
    )
    .expect("approved graph candidate insert should succeed");
    conn.execute(
        "INSERT INTO pending_observations (status, created_at_epoch) VALUES ('pending', 100)",
        [],
    )
    .expect("pending insert should succeed");
    conn.execute(
        "INSERT INTO pending_observations (status, created_at_epoch) VALUES ('pending', 120)",
        [],
    )
    .expect("second pending insert should succeed");
    conn.execute(
        "UPDATE pending_observations SET next_retry_epoch = strftime('%s', 'now') + 3600 WHERE id = 2",
        [],
    )
    .expect("delayed pending update should succeed");
    conn.execute(
        "INSERT INTO pending_observations (status, created_at_epoch, lease_owner, lease_expires_epoch)
         VALUES ('processing', 130, 'worker-a', strftime('%s', 'now') - 1)",
        [],
    )
    .expect("processing pending insert should succeed");
    conn.execute(
        "INSERT INTO pending_observations (status, created_at_epoch) VALUES ('failed', 140)",
        [],
    )
    .expect("failed pending insert should succeed");
    conn.execute(
        "INSERT INTO jobs (state, lease_expires_epoch) VALUES ('pending', NULL)",
        [],
    )
    .expect("pending job insert should succeed");
    conn.execute(
        "INSERT INTO jobs (state, lease_expires_epoch) VALUES ('processing', 0)",
        [],
    )
    .expect("stuck job insert should succeed");
    conn.execute(
        "INSERT INTO jobs (state, lease_expires_epoch) VALUES ('failed', NULL)",
        [],
    )
    .expect("failed job insert should succeed");
    conn.execute(
        "INSERT INTO worker_heartbeats (owner, pid, started_at_epoch, updated_at_epoch)
         VALUES ('worker-a', ?1, strftime('%s', 'now') - 10, strftime('%s', 'now') - 10)",
        [i64::from(std::process::id())],
    )
    .expect("heartbeat insert should succeed");

    let system = query_system_stats(&conn).expect("system stats should load");
    assert_eq!(
        system,
        SystemStats {
            active_memories: 3,
            active_observations: 1,
            session_summaries: 1,
            raw_messages: 0,
            raw_ingest_failures: 1,
            raw_ingest_parse_errors: 2,
            raw_ingest_insert_errors: 1,
            latest_raw_ingest_failure_epoch: Some(160),
            latest_raw_ingest_failure_kind: Some("parse_errors".to_string()),
            latest_raw_ingest_failure_path: Some("/bad/transcript.jsonl".to_string()),
            latest_raw_ingest_failure_message: Some("bad jsonl".to_string()),
            captured_events: 1,
            capture_drop_events: 1,
            actionable_capture_drops: 0,
            unrecovered_capture_spills: 0,
            latest_capture_drop_epoch: Some(170),
            latest_capture_drop_reason: Some("codex_bash_disabled".to_string()),
            latest_capture_drop_detail: Some("Codex Bash capture disabled".to_string()),
            pending_extraction_tasks: 1,
            processing_extraction_tasks: 1,
            expired_processing_extraction_tasks: 1,
            failed_extraction_tasks: 1,
            oldest_pending_extraction_epoch: Some(90),
            pending_memory_candidates: 1,
            pending_graph_candidates: 2,
            pending_observations: 2,
            ready_pending_observations: 1,
            delayed_pending_observations: 1,
            processing_pending_observations: 1,
            expired_processing_pending_observations: 1,
            failed_pending_observations: 1,
            oldest_ready_pending_epoch: Some(100),
            pending_jobs: 1,
            processing_jobs: 1,
            failed_jobs: 1,
            stuck_jobs: 1,
            worker_daemon_healthy: true,
            worker_heartbeat_owner: Some("worker-a".to_string()),
            worker_heartbeat_age_secs: system.worker_heartbeat_age_secs,
        }
    );
    assert!(
        system.worker_heartbeat_age_secs.unwrap_or_default() <= 20,
        "heartbeat age should be recent"
    );

    let daily = query_daily_activity_stats(&conn, 180).expect("daily stats should load");
    assert_eq!(
        daily,
        DailyActivityStats {
            memories: 4,
            observations: 1,
        }
    );

    let top_projects = query_top_projects(&conn, 5).expect("top projects should load");
    assert_eq!(
        top_projects,
        vec![
            ProjectCount {
                project: "alpha".to_string(),
                count: 1,
            },
            ProjectCount {
                project: "beta".to_string(),
                count: 1,
            },
            ProjectCount {
                project: "delta".to_string(),
                count: 1,
            },
        ]
    );
}

#[test]
fn query_memory_facts_stats_excludes_expired_source_memories() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_stats_schema(&conn);

    conn.execute_batch(
        "INSERT INTO memories (id, project, status, created_at_epoch, expires_at_epoch)
         VALUES
            (1, 'alpha', 'active', 100, NULL),
            (2, 'alpha', 'active', 110, CAST(strftime('%s', 'now') AS INTEGER) + 3600),
            (3, 'alpha', 'active', 120, CAST(strftime('%s', 'now') AS INTEGER) - 1),
            (4, 'alpha', 'archived', 130, NULL);
         INSERT INTO memory_facts (status, valid_from_epoch, source_memory_id)
         VALUES
            ('active', 100, 1),
            ('active', 110, 2),
            ('active', 120, 3),
            ('active', 130, 4),
            ('active', NULL, 1),
            ('stale', 140, 1);",
    )?;

    let stats = query_memory_facts_stats(&conn)?;

    assert!(stats.table_exists);
    assert_eq!(stats.total, 6);
    assert_eq!(stats.active_memories, 2);
    assert_eq!(stats.retrieval_eligible, 2);
    Ok(())
}

#[test]
fn query_system_stats_reports_daemon_heartbeat_not_once() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_stats_schema(&conn);
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO worker_heartbeats (owner, pid, started_at_epoch, updated_at_epoch)
         VALUES ('worker-daemon-stats', ?1, ?2, ?2)",
        (i64::from(std::process::id()), now - 10),
    )?;
    conn.execute(
        "INSERT INTO worker_heartbeats (owner, pid, started_at_epoch, updated_at_epoch)
         VALUES ('worker-once-stats', ?1, ?2, ?2)",
        (i64::from(std::process::id()), now),
    )?;

    let system = query_system_stats(&conn)?;

    assert!(system.worker_daemon_healthy);
    assert_eq!(
        system.worker_heartbeat_owner.as_deref(),
        Some("worker-daemon-stats")
    );
    assert!(
        system.worker_heartbeat_age_secs.unwrap_or_default() >= 10,
        "reported age should come from daemon heartbeat"
    );
    Ok(())
}

#[test]
fn query_system_stats_defaults_raw_ingest_failures_when_table_is_absent() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_stats_schema(&conn);
    conn.execute("DROP TABLE raw_ingest_failures", [])?;

    let system = query_system_stats(&conn)?;

    assert_eq!(system.raw_ingest_failures, 0);
    assert_eq!(system.raw_ingest_parse_errors, 0);
    assert_eq!(system.raw_ingest_insert_errors, 0);
    assert_eq!(system.latest_raw_ingest_failure_epoch, None);
    Ok(())
}

#[test]
fn query_system_stats_defaults_capture_drops_when_table_is_absent() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_stats_schema(&conn);
    conn.execute("DROP TABLE capture_drop_events", [])?;

    let system = query_system_stats(&conn)?;

    assert_eq!(system.capture_drop_events, 0);
    assert_eq!(system.actionable_capture_drops, 0);
    assert_eq!(system.unrecovered_capture_spills, 0);
    assert_eq!(system.latest_capture_drop_epoch, None);
    assert_eq!(system.latest_capture_drop_reason, None);
    Ok(())
}

fn insert_usage(
    conn: &Connection,
    project: &str,
    created_at_epoch: i64,
    input_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: i64,
    cache_read_tokens: i64,
    estimated_cost_usd: f64,
) {
    insert_usage_with_source(
        conn,
        Some(project),
        created_at_epoch,
        "codex-cli",
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cache_read_tokens,
        estimated_cost_usd,
        "codex_log",
        "remem_static",
    );
}

fn insert_usage_with_source(
    conn: &Connection,
    project: Option<&str>,
    created_at_epoch: i64,
    executor: &str,
    input_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: i64,
    cache_read_tokens: i64,
    estimated_cost_usd: f64,
    usage_source: &str,
    pricing_source: &str,
) {
    conn.execute(
        "INSERT INTO ai_usage_events
         (created_at, created_at_epoch, project, operation, executor, model,
          input_tokens, output_tokens, reasoning_tokens, cache_read_tokens, total_tokens,
          estimated_cost_usd, usage_source, pricing_source)
         VALUES ('2026-01-01T00:00:00Z', ?1, ?2, 'summary', ?3, 'codex-default',
                 ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            created_at_epoch,
            project,
            executor,
            input_tokens,
            output_tokens,
            reasoning_tokens,
            cache_read_tokens,
            input_tokens + output_tokens + reasoning_tokens + cache_read_tokens,
            estimated_cost_usd,
            usage_source,
            pricing_source
        ],
    )
    .expect("usage insert should succeed");
}

#[test]
fn query_ai_usage_groups_daily_and_weekly_token_costs() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    setup_stats_schema(&conn);

    let jan_05_2026 = 1_767_571_200;
    let jan_06_2026 = 1_767_657_600;
    let jan_12_2026 = 1_768_176_000;

    insert_usage(&conn, "alpha", jan_05_2026, 100, 40, 10, 50, 0.001);
    insert_usage(&conn, "alpha", jan_05_2026 + 60, 200, 60, 20, 80, 0.002);
    insert_usage(&conn, "alpha", jan_06_2026, 300, 80, 30, 120, 0.003);
    insert_usage(&conn, "beta", jan_12_2026, 500, 100, 40, 160, 0.005);

    let alpha_totals = query_ai_usage_totals(&conn, Some(jan_05_2026), Some("alpha"))
        .expect("usage totals should load");
    assert_eq!(
        alpha_totals,
        AiUsageTotals {
            calls: 3,
            input_tokens: 600,
            output_tokens: 180,
            reasoning_tokens: 60,
            cache_creation_tokens: 0,
            cache_read_tokens: 250,
            total_tokens: 1090,
            estimated_cost_usd: 0.006,
        }
    );

    let alpha_sources = query_ai_usage_source_totals(&conn, Some(jan_05_2026), Some("alpha"))
        .expect("usage source totals should load");
    assert_eq!(
        alpha_sources,
        vec![AiUsageSourceTotals {
            usage_source: "codex_log".to_string(),
            pricing_source: "remem_static".to_string(),
            calls: 3,
            total_tokens: 1090,
            estimated_cost_usd: 0.006,
        }]
    );

    let alpha_breakdown = query_ai_usage_breakdown(&conn, Some(jan_05_2026), Some("alpha"), 10)?;
    assert_eq!(
        alpha_breakdown,
        vec![AiUsageBreakdown {
            project: Some("alpha".to_string()),
            executor: "codex-cli".to_string(),
            usage_source: "codex_log".to_string(),
            pricing_source: "remem_static".to_string(),
            calls: 3,
            total_tokens: 1090,
            estimated_cost_usd: 0.006,
        }]
    );

    let daily = query_daily_ai_usage(&conn, jan_05_2026, Some("alpha"), 14)
        .expect("daily usage should load");
    assert_eq!(
        daily,
        vec![
            DailyAiUsage {
                day: "2026-01-06".to_string(),
                calls: 1,
                input_tokens: 300,
                output_tokens: 80,
                reasoning_tokens: 30,
                cache_creation_tokens: 0,
                cache_read_tokens: 120,
                total_tokens: 530,
                estimated_cost_usd: 0.003,
            },
            DailyAiUsage {
                day: "2026-01-05".to_string(),
                calls: 2,
                input_tokens: 300,
                output_tokens: 100,
                reasoning_tokens: 30,
                cache_creation_tokens: 0,
                cache_read_tokens: 130,
                total_tokens: 560,
                estimated_cost_usd: 0.003,
            },
        ]
    );

    let weekly =
        query_weekly_ai_usage(&conn, jan_05_2026, None, 8).expect("weekly usage should load");
    assert_eq!(
        weekly,
        vec![
            WeeklyAiUsage {
                week: "2026-W02".to_string(),
                calls: 1,
                input_tokens: 500,
                output_tokens: 100,
                reasoning_tokens: 40,
                cache_creation_tokens: 0,
                cache_read_tokens: 160,
                total_tokens: 800,
                estimated_cost_usd: 0.005,
            },
            WeeklyAiUsage {
                week: "2026-W01".to_string(),
                calls: 3,
                input_tokens: 600,
                output_tokens: 180,
                reasoning_tokens: 60,
                cache_creation_tokens: 0,
                cache_read_tokens: 250,
                total_tokens: 1090,
                estimated_cost_usd: 0.006,
            },
        ]
    );
    Ok(())
}

#[test]
fn query_ai_usage_breakdown_exposes_project_executor_and_source() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    setup_stats_schema(&conn);

    let jan_05_2026 = 1_767_571_200;
    insert_usage_with_source(
        &conn,
        Some("/Users/lifcc/.remem"),
        jan_05_2026,
        "cli",
        900,
        100,
        0,
        0,
        0.003,
        "text_estimate",
        "remem_static",
    );
    insert_usage_with_source(
        &conn,
        Some("alpha"),
        jan_05_2026 + 60,
        "codex-cli",
        100,
        50,
        0,
        25,
        0.001,
        "codex_log",
        "remem_static",
    );
    insert_usage_with_source(
        &conn,
        None,
        jan_05_2026 + 120,
        "http",
        80,
        20,
        0,
        0,
        0.0005,
        "anthropic_usage",
        "remem_static",
    );

    let breakdown = query_ai_usage_breakdown(&conn, Some(jan_05_2026), None, 10)?;

    assert_eq!(
        breakdown,
        vec![
            AiUsageBreakdown {
                project: Some("/Users/lifcc/.remem".to_string()),
                executor: "cli".to_string(),
                usage_source: "text_estimate".to_string(),
                pricing_source: "remem_static".to_string(),
                calls: 1,
                total_tokens: 1000,
                estimated_cost_usd: 0.003,
            },
            AiUsageBreakdown {
                project: Some("alpha".to_string()),
                executor: "codex-cli".to_string(),
                usage_source: "codex_log".to_string(),
                pricing_source: "remem_static".to_string(),
                calls: 1,
                total_tokens: 175,
                estimated_cost_usd: 0.001,
            },
            AiUsageBreakdown {
                project: None,
                executor: "http".to_string(),
                usage_source: "anthropic_usage".to_string(),
                pricing_source: "remem_static".to_string(),
                calls: 1,
                total_tokens: 100,
                estimated_cost_usd: 0.0005,
            },
        ]
    );

    let limited = query_ai_usage_breakdown(&conn, Some(jan_05_2026), None, 1)?;
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].project.as_deref(), Some("/Users/lifcc/.remem"));

    let empty = query_ai_usage_breakdown(&conn, Some(jan_05_2026), None, 0)?;
    assert!(empty.is_empty());
    Ok(())
}

#[test]
fn query_candidate_promotion_stats_groups_by_status_and_block_reason() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_stats_schema(&conn);

    let now = 10_000_000;
    let recent = now - 1_000;
    let old = now - 8 * 24 * 3600;
    conn.execute_batch(&format!(
        "INSERT INTO memory_candidates (review_status, auto_promote_block_reason, created_at_epoch) VALUES
            ('auto_promoted', NULL, {recent}),
            ('auto_promoted', NULL, {old}),
            ('auto_promoted', NULL, {old}),
            ('pending_review', 'no_supporting_source_observation', {recent}),
            ('pending_review', 'no_supporting_source_observation', {recent}),
            ('pending_review', 'no_supporting_source_observation', {old}),
            ('pending_review', 'no_supporting_source_observation', {old}),
            ('pending_review', 'confidence_below_threshold', {old});"
    ))?;

    let stats = query_candidate_promotion_stats(&conn, now)?;

    assert_eq!(
        stats,
        vec![
            CandidatePromotionStat {
                review_status: "pending_review".to_string(),
                block_reason: Some("no_supporting_source_observation".to_string()),
                total: 4,
                last_7_days: 2,
            },
            CandidatePromotionStat {
                review_status: "auto_promoted".to_string(),
                block_reason: None,
                total: 3,
                last_7_days: 1,
            },
            CandidatePromotionStat {
                review_status: "pending_review".to_string(),
                block_reason: Some("confidence_below_threshold".to_string()),
                total: 1,
                last_7_days: 0,
            },
        ]
    );
    Ok(())
}
