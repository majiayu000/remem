use anyhow::Result;
use rusqlite::Connection;

use super::{
    query_observability_report, CountBucket, CURRENT_MEMORY_CONTRACT_SPEC_PATH,
    OBSERVABILITY_SCHEMA_VERSION,
};

fn setup_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE memories (
            id INTEGER PRIMARY KEY,
            session_id TEXT,
            project TEXT NOT NULL,
            topic_key TEXT,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            memory_type TEXT NOT NULL,
            files TEXT,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            status TEXT NOT NULL,
            branch TEXT,
            scope TEXT DEFAULT 'project',
            expires_at_epoch INTEGER,
            state_key_id INTEGER
        );
        CREATE TABLE memory_state_keys (
            id INTEGER PRIMARY KEY,
            current_memory_id INTEGER
        );
        CREATE TABLE observations (
            id INTEGER PRIMARY KEY,
            project TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL
        );
        CREATE TABLE session_summaries (id INTEGER PRIMARY KEY);
        CREATE TABLE raw_messages (
            id INTEGER PRIMARY KEY,
            created_at_epoch INTEGER NOT NULL
        );
        CREATE TABLE captured_events (
            id INTEGER PRIMARY KEY,
            created_at_epoch INTEGER NOT NULL,
            inserted_at_epoch INTEGER NOT NULL
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
            lease_expires_epoch INTEGER,
            replay_range_id INTEGER
        );
        CREATE TABLE extraction_replay_ranges (
            id INTEGER PRIMARY KEY,
            status TEXT NOT NULL
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
        CREATE TABLE memory_facts (
            id INTEGER PRIMARY KEY,
            project TEXT NOT NULL,
            subject TEXT NOT NULL,
            predicate TEXT NOT NULL,
            object TEXT NOT NULL,
            valid_from_epoch INTEGER,
            valid_to_epoch INTEGER,
            learned_at_epoch INTEGER NOT NULL,
            source_memory_id INTEGER,
            source_observation_id INTEGER,
            source_event_ids TEXT NOT NULL DEFAULT '[]',
            confidence REAL NOT NULL,
            supersedes_fact_id INTEGER,
            status TEXT NOT NULL DEFAULT 'active',
            invalidated_at_epoch INTEGER,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL
        );
        CREATE TABLE context_injections (
            id INTEGER PRIMARY KEY,
            host TEXT NOT NULL,
            project TEXT NOT NULL,
            injection_key TEXT NOT NULL,
            session_id TEXT,
            transcript_path TEXT,
            hook_source TEXT,
            context_hash TEXT NOT NULL,
            output_mode TEXT NOT NULL,
            output_chars INTEGER NOT NULL,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            last_emitted_epoch INTEGER NOT NULL,
            emit_count INTEGER NOT NULL DEFAULT 1,
            suppress_count INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE context_injection_items (
            id INTEGER PRIMARY KEY,
            injection_run_id TEXT NOT NULL,
            host TEXT NOT NULL,
            project TEXT NOT NULL,
            session_id TEXT,
            injection_key TEXT NOT NULL,
            hook_source TEXT,
            context_hash TEXT,
            output_mode TEXT NOT NULL,
            decision TEXT NOT NULL,
            item_kind TEXT NOT NULL,
            item_id INTEGER,
            memory_id INTEGER,
            channel TEXT NOT NULL,
            score REAL,
            render_order INTEGER,
            status TEXT NOT NULL,
            drop_reason TEXT,
            title TEXT,
            provenance TEXT,
            staleness TEXT,
            injected_at_epoch INTEGER NOT NULL
        );
        CREATE TABLE memory_citation_events (
            id INTEGER PRIMARY KEY,
            host TEXT NOT NULL,
            project TEXT NOT NULL,
            session_id TEXT NOT NULL,
            source TEXT NOT NULL,
            message_hash TEXT NOT NULL,
            citation_line_present INTEGER NOT NULL DEFAULT 0,
            parsed_count INTEGER NOT NULL DEFAULT 0,
            matched_count INTEGER NOT NULL DEFAULT 0,
            inserted_count INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL
        );
        CREATE TABLE memory_usage_events (
            id INTEGER PRIMARY KEY,
            citation_event_id INTEGER NOT NULL,
            host TEXT NOT NULL,
            project TEXT NOT NULL,
            session_id TEXT NOT NULL,
            source TEXT NOT NULL,
            message_hash TEXT NOT NULL,
            memory_id INTEGER NOT NULL,
            context_injection_item_id INTEGER,
            created_at_epoch INTEGER NOT NULL
        );",
    )?;
    Ok(())
}

fn bucket_count(buckets: &[CountBucket], value: &str) -> i64 {
    buckets
        .iter()
        .find(|bucket| bucket.value == value)
        .map(|bucket| bucket.count)
        .unwrap_or_default()
}

#[test]
fn observability_report_exposes_current_memory_contract_metrics() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_schema(&conn)?;
    conn.execute_batch(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope, expires_at_epoch)
         VALUES
         (1, NULL, 'proj', NULL, 'Tracked', 'body', 'decision', NULL,
          10, 20, 'active', NULL, 'project', NULL);
         INSERT INTO observations (project, status, created_at_epoch)
         VALUES ('proj', 'active', 21);
         INSERT INTO captured_events (id, created_at_epoch, inserted_at_epoch)
         VALUES (1, 30, 30);
         INSERT INTO memory_candidates (review_status, created_at_epoch)
         VALUES ('pending_review', 40);
         INSERT INTO context_injections
         (host, project, injection_key, session_id, context_hash, output_mode, output_chars,
          created_at_epoch, updated_at_epoch, last_emitted_epoch, emit_count, suppress_count)
         VALUES
         ('codex-cli', 'proj', 'key-a', 'sess', 'hash', 'full', 1200, 50, 60, 60, 2, 1);
         INSERT INTO context_injection_items
         (injection_run_id, host, project, session_id, injection_key, output_mode, decision,
          item_kind, memory_id, channel, status, drop_reason, title, provenance, staleness,
          injected_at_epoch)
         VALUES
         ('run-1', 'codex-cli', 'proj', 'sess', 'key-a', 'full', 'emit',
          'memory', 1, 'core', 'injected', NULL, 'Tracked', 'src=memory',
          'status=active; staleness=fresh; source_anchor=untracked', 60),
         ('run-1', 'codex-cli', 'proj', 'sess', 'key-a', 'full', 'emit',
          'memory', 2, 'index', 'dropped', 'section_budget', 'Drop', 'src=memory',
          'status=active; staleness=old; source_anchor=verify-before-trust', 60);
         INSERT INTO memory_citation_events
         (host, project, session_id, source, message_hash, citation_line_present,
          parsed_count, matched_count, inserted_count, status, created_at_epoch)
         VALUES
         ('codex-cli', 'proj', 'sess', 'stop_citation', 'm1', 1, 1, 1, 1, 'recorded', 70),
         ('codex-cli', 'proj', 'sess', 'stop_citation', 'm2', 1, 0, 0, 0, 'recorded', 71);
         UPDATE memory_citation_events SET status = 'matched' WHERE message_hash = 'm1';
         UPDATE memory_citation_events SET status = 'no_citation' WHERE message_hash = 'm2';
         INSERT INTO memory_usage_events
         (citation_event_id, host, project, session_id, source, message_hash, memory_id,
          context_injection_item_id, created_at_epoch)
         VALUES
         (1, 'codex-cli', 'proj', 'sess', 'stop_citation', 'm1', 1, 1, 72);
         INSERT INTO memory_facts
         (project, subject, predicate, object, valid_from_epoch, valid_to_epoch, learned_at_epoch,
          source_memory_id, source_observation_id, source_event_ids, confidence, status,
          created_at_epoch, updated_at_epoch)
         VALUES
         ('proj', 'memory:1', 'uses_file', 'src/lib.rs', 10, NULL, 20, 1, NULL, '[]',
          0.9, 'active', 20, 20);",
    )?;

    let report = query_observability_report(&conn, 100)?;

    assert_eq!(report.schema_version, OBSERVABILITY_SCHEMA_VERSION);
    assert_eq!(report.spec_path, CURRENT_MEMORY_CONTRACT_SPEC_PATH);
    assert_eq!(report.metrics.capture.captured_events, 1);
    assert_eq!(report.metrics.promotion.candidates, 1);
    assert_eq!(report.metrics.context_injection.output_emit_count, 2);
    assert_eq!(report.metrics.context_injection.item_rows, 2);
    assert_eq!(report.metrics.usage_feedback.citation_events, 2);
    assert_eq!(
        report.metrics.usage_feedback.citation_line_present_events,
        2
    );
    assert_eq!(report.metrics.usage_feedback.no_citation_events, 1);
    assert_eq!(report.metrics.usage_feedback.unmatched_events, 0);
    assert_eq!(report.metrics.usage_feedback.matched_events, 1);
    assert_eq!(report.metrics.usage_feedback.usage_events, 1);
    assert_eq!(report.metrics.temporal_facts.total_rows, 1);
    assert_eq!(report.metrics.temporal_facts.retrieval_eligible_rows, 1);
    assert_eq!(report.metrics.staleness.total_memories, 1);
    let worker_check = report
        .checks
        .iter()
        .find(|check| check.code == "worker_daemon_not_healthy")
        .ok_or_else(|| anyhow::anyhow!("missing worker health check"))?;
    assert_eq!(
        worker_check.metrics.get("heartbeat_owner_present"),
        Some(&0)
    );
    assert!(!worker_check.metrics.contains_key("heartbeat_age_secs"));
    let pending_review_check = report
        .checks
        .iter()
        .find(|check| check.code == "promotion_funnel_all_pending_review")
        .ok_or_else(|| anyhow::anyhow!("missing all-pending promotion check"))?;
    assert!(pending_review_check
        .actions
        .iter()
        .any(|action| { action == "run `remem review list --limit 20`" }));
    assert!(pending_review_check.actions.iter().any(|action| {
        action.contains("remem review approve <id>")
            && action.contains("non-duplicate")
            && action.contains("linked temporal fact")
    }));
    Ok(())
}

#[test]
fn observability_report_keeps_citation_counts_when_usage_table_is_missing() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_schema(&conn)?;
    conn.execute_batch(
        "DROP TABLE memory_usage_events;
         INSERT INTO memory_citation_events
         (host, project, session_id, source, message_hash, citation_line_present,
          parsed_count, matched_count, inserted_count, status, created_at_epoch)
         VALUES
         ('codex-cli', 'proj', 'sess', 'stop_citation', 'm1', 1, 1, 0, 0, 'unmatched', 70),
         ('codex-cli', 'proj', 'sess', 'stop_citation', 'm2', 1, 0, 0, 0, 'no_citation', 71);",
    )?;

    let report = query_observability_report(&conn, 100)?;

    assert!(report.metrics.usage_feedback.citation_table_exists);
    assert!(!report.metrics.usage_feedback.usage_table_exists);
    assert_eq!(report.metrics.usage_feedback.citation_events, 2);
    assert_eq!(
        report.metrics.usage_feedback.citation_line_present_events,
        2
    );
    assert_eq!(report.metrics.usage_feedback.no_citation_events, 1);
    assert_eq!(report.metrics.usage_feedback.unmatched_events, 1);
    assert_eq!(report.metrics.usage_feedback.usage_events, 0);
    assert!(report.checks.iter().any(|check| {
        check.code == "memory_usage_feedback_missing"
            && check.metrics.get("citation_table_exists") == Some(&1)
            && check.metrics.get("usage_table_exists") == Some(&0)
    }));
    Ok(())
}

#[test]
fn observability_report_warns_on_missing_optional_truth_tables() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_schema(&conn)?;
    conn.execute_batch(
        "DROP TABLE context_injection_items;
         DROP TABLE memory_citation_events;",
    )?;

    let report = query_observability_report(&conn, 100)?;

    assert!(report
        .checks
        .iter()
        .any(|check| check.code == "context_injection_audit_missing" && check.severity == "warn"));
    assert!(report
        .checks
        .iter()
        .any(|check| check.code == "memory_usage_feedback_missing" && check.severity == "warn"));
    assert!(!report.metrics.context_injection.item_table_exists);
    assert!(!report.metrics.usage_feedback.citation_table_exists);
    Ok(())
}

#[test]
fn staleness_metrics_qualify_state_key_current_filter() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_schema(&conn)?;
    conn.execute_batch(
        "INSERT INTO memory_state_keys (id, current_memory_id)
         VALUES (200, 1);
         INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope, expires_at_epoch,
          state_key_id)
         VALUES
         (1, NULL, 'proj', NULL, 'Tracked', 'body', 'decision', NULL,
          10, 20, 'active', NULL, 'project', NULL, 200);",
    )?;

    let report = query_observability_report(&conn, 100)?;

    assert_eq!(report.metrics.staleness.total_memories, 1);
    Ok(())
}

#[test]
fn no_citation_usage_feedback_is_not_reported_as_unmatched() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_schema(&conn)?;
    conn.execute_batch(
        "INSERT INTO memory_citation_events
         (host, project, session_id, source, message_hash, citation_line_present,
          parsed_count, matched_count, inserted_count, status, created_at_epoch)
         VALUES
         ('codex-cli', 'proj', 'sess', 'stop_citation', 'm1', 0, 0, 0, 0,
          'no_citation', 70);",
    )?;

    let report = query_observability_report(&conn, 100)?;

    assert_eq!(report.metrics.usage_feedback.no_citation_events, 1);
    assert_eq!(report.metrics.usage_feedback.unmatched_events, 0);
    assert!(!report
        .checks
        .iter()
        .any(|check| check.code == "memory_usage_feedback_no_matches"));
    Ok(())
}

#[test]
fn null_context_staleness_is_counted_as_unknown() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_schema(&conn)?;
    conn.execute_batch(
        "INSERT INTO context_injection_items
         (injection_run_id, host, project, session_id, injection_key, output_mode, decision,
          item_kind, memory_id, channel, status, drop_reason, title, provenance, staleness,
          injected_at_epoch)
         VALUES
         ('run-1', 'codex-cli', 'proj', 'sess', 'key-a', 'full', 'emit',
          'memory', 1, 'core', 'injected', NULL, 'Tracked', 'src=memory', NULL, 60);",
    )?;

    let report = query_observability_report(&conn, 100)?;

    assert_eq!(
        bucket_count(
            &report
                .metrics
                .context_injection
                .item_staleness_source_anchors,
            "unknown",
        ),
        1
    );
    assert_eq!(
        bucket_count(
            &report.metrics.context_injection.item_staleness_ages,
            "unknown",
        ),
        1
    );
    Ok(())
}

#[test]
fn captured_events_without_observations_warn_on_promotion_funnel() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_schema(&conn)?;
    conn.execute_batch(
        "INSERT INTO captured_events (id, created_at_epoch, inserted_at_epoch)
         VALUES (1, 30, 30);",
    )?;

    let report = query_observability_report(&conn, 100)?;

    assert!(report.checks.iter().any(|check| {
        check.code == "promotion_funnel_no_observations"
            && check.metrics.get("captured_events") == Some(&1)
            && check.metrics.get("observations") == Some(&0)
    }));
    Ok(())
}
