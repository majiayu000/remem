use anyhow::Result;
use rusqlite::{params, Connection};

use super::{dry_run_pending, run_migrations, MIGRATIONS};

#[test]
fn validate_schema_invariants_is_clean_after_current_migrations() -> Result<()> {
    let conn = Connection::open_in_memory()?;

    run_migrations(&conn)?;

    let errors = super::validate_schema_invariants(&conn)?;
    assert!(errors.is_empty(), "unexpected schema drift: {errors:?}");
    Ok(())
}

#[test]
fn run_migrations_repairs_v022_schema_drift() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_current_schema_missing_versions(&conn, &[22])?;

    run_migrations(&conn)?;

    assert!(conn
        .prepare("SELECT id, state_key FROM memory_state_keys LIMIT 0")
        .is_ok());
    assert!(conn
        .prepare("SELECT state_key_id FROM memories LIMIT 0")
        .is_ok());
    assert!(conn
        .prepare(
            "SELECT state_key, state_key_confidence, state_key_reason
             FROM memory_candidates LIMIT 0"
        )
        .is_ok());
    for index in [
        "idx_memory_state_keys_owner",
        "idx_memory_state_keys_current",
        "idx_memories_state_key_id",
        "idx_memory_candidates_state_key",
    ] {
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1",
                [index],
                |_| Ok(true),
            )
            .unwrap_or(false);
        assert!(exists, "{index} index should be repaired");
    }
    Ok(())
}

#[test]
fn dry_run_pending_reports_v022_schema_drift() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_current_schema_missing_versions(&conn, &[22])?;

    let result = dry_run_pending(&conn)?;

    assert_eq!(result.pending_count, 0);
    let error = result
        .error
        .expect("schema drift must be reported even when no migrations are pending");
    assert!(error.contains("schema drift"));
    assert!(error.contains("v022_memory_state_keys marked applied"));
    assert!(error.contains("table memory_state_keys"));
    Ok(())
}

#[test]
fn dry_run_pending_reports_post_v022_schema_drift() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_current_schema_missing_versions(&conn, &[45])?;

    let result = dry_run_pending(&conn)?;

    assert_eq!(result.pending_count, 0);
    let error = result
        .error
        .expect("post-v022 schema drift must be reported when no migrations are pending");
    assert!(error.contains("schema drift"), "got: {error}");
    assert!(error.contains("v045_memory_usage_columns"), "got: {error}");
    assert!(
        error.contains("column memories.access_count"),
        "got: {error}"
    );
    assert!(
        error.contains("table memory_citation_events"),
        "got: {error}"
    );
    Ok(())
}

#[test]
fn dry_run_pending_reports_v059_review_metadata_schema_drift() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_current_schema_missing_versions(&conn, &[59])?;

    let result = dry_run_pending(&conn)?;

    assert_eq!(result.pending_count, 0);
    let error = result
        .error
        .expect("review metadata schema drift must be reported");
    assert!(
        error.contains("v059_candidate_review_metadata"),
        "got: {error}"
    );
    assert!(
        error.contains("column memory_candidates.review_actor"),
        "got: {error}"
    );
    assert!(
        error.contains("column memory_candidates.reviewed_at_epoch"),
        "got: {error}"
    );
    assert!(
        error.contains("index idx_memory_candidates_review_status_created"),
        "got: {error}"
    );
    Ok(())
}

#[test]
fn dry_run_pending_reports_v060_memory_poisoning_schema_drift() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_current_schema_missing_versions(&conn, &[60])?;

    let result = dry_run_pending(&conn)?;

    assert_eq!(result.pending_count, 0);
    let error = result
        .error
        .expect("memory poisoning schema drift must be reported");
    assert!(
        error.contains("v060_memory_poisoning_defense"),
        "got: {error}"
    );
    assert!(
        error.contains("column memory_candidates.source_trust_class"),
        "got: {error}"
    );
    assert!(
        error.contains("column memories.source_trust_class"),
        "got: {error}"
    );
    assert!(
        error.contains("index idx_memory_candidates_quarantine"),
        "got: {error}"
    );
    Ok(())
}

#[test]
fn dry_run_pending_reports_v061_memory_poisoning_drop_schema_drift() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_current_schema_missing_versions(&conn, &[61])?;

    let result = dry_run_pending(&conn)?;

    assert_eq!(result.pending_count, 0);
    let error = result
        .error
        .expect("memory poisoning injection drop schema drift must be reported");
    assert!(
        error.contains("v061_memory_poisoning_injection_drops"),
        "got: {error}"
    );
    assert!(
        error.contains("table memory_poisoning_injection_drops"),
        "got: {error}"
    );
    assert!(
        error.contains("index idx_memory_poisoning_drops_created"),
        "got: {error}"
    );
    Ok(())
}

#[test]
fn dry_run_pending_reports_v063_procedure_exports_schema_drift() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_current_schema_missing_versions(&conn, &[63])?;

    let result = dry_run_pending(&conn)?;

    assert_eq!(result.pending_count, 0);
    let error = result.error.ok_or_else(|| {
        anyhow::anyhow!("procedure export registry schema drift must be reported")
    })?;
    assert!(error.contains("v063_procedure_exports"), "got: {error}");
    assert!(error.contains("table procedure_exports"), "got: {error}");
    assert!(
        error.contains("index idx_procedure_exports_project"),
        "got: {error}"
    );
    assert!(
        error.contains("index idx_procedure_exports_memory"),
        "got: {error}"
    );
    Ok(())
}

#[test]
fn dry_run_pending_reports_v065_preference_reinforcement_schema_drift() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_current_schema_missing_versions(&conn, &[65])?;

    let result = dry_run_pending(&conn)?;

    assert_eq!(result.pending_count, 0);
    let error = result
        .error
        .ok_or_else(|| anyhow::anyhow!("preference reinforcement drift must be reported"))?;
    assert!(
        error.contains("v065_preference_reinforcement"),
        "got: {error}"
    );
    assert!(
        error.contains("column memory_preference_reinforcements.machine_checkable"),
        "got: {error}"
    );
    assert!(
        error.contains("column memory_preference_reinforcements.risk_class"),
        "got: {error}"
    );
    assert!(
        error.contains("index idx_memory_preference_reinforcements_eligible"),
        "got: {error}"
    );
    Ok(())
}

#[test]
fn dry_run_pending_reports_v066_session_rollup_evidence_checkpoint_drift() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_current_schema_missing_versions(&conn, &[66])?;

    let result = dry_run_pending(&conn)?;

    assert_eq!(result.pending_count, 0);
    let error = result
        .error
        .ok_or_else(|| anyhow::anyhow!("session rollup evidence drift must be reported"))?;
    assert!(
        error.contains("v066_session_rollup_evidence_checkpoint"),
        "got: {error}"
    );
    assert!(
        error.contains("column session_summaries.transcript_evidence_json"),
        "got: {error}"
    );
    assert!(
        error.contains("column session_summaries.raw_archive_completed_at_epoch"),
        "got: {error}"
    );
    Ok(())
}

#[test]
fn dry_run_pending_reports_v068_session_rollup_followup_checkpoint_drift() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_current_schema_missing_versions(&conn, &[68])?;

    let result = dry_run_pending(&conn)?;

    assert_eq!(result.pending_count, 0);
    let error = result
        .error
        .ok_or_else(|| anyhow::anyhow!("session rollup follow-up drift must be reported"))?;
    assert!(
        error.contains("v068_session_rollup_followup_checkpoint"),
        "got: {error}"
    );
    assert!(
        error.contains("column session_summaries.followup_scheduling_completed_at_epoch"),
        "got: {error}"
    );
    assert!(
        error.contains("column session_summaries.followup_scheduling_state"),
        "got: {error}"
    );
    assert!(
        error.contains("column session_summaries.followup_compress_job_id"),
        "got: {error}"
    );
    assert!(
        error.contains("column session_summaries.followup_dream_disposition"),
        "got: {error}"
    );
    assert!(
        error.contains("column session_summaries.followup_dream_job_id"),
        "got: {error}"
    );
    Ok(())
}

#[test]
fn v068_marks_historical_exact_ranges_legacy_unknown_without_inventing_jobs() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_schema_with_pending_migrations_from(&conn, 68)?;
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, session_row_id,
          covered_from_event_id, covered_to_event_id)
         VALUES ('historical-rollup', '/tmp/remem', 7, 10, 20)",
        [],
    )?;
    let compress_job_id = crate::db::enqueue_job(
        &conn,
        "codex-cli",
        crate::db::JobType::Compress,
        "/tmp/remem",
        None,
        "{}",
        200,
    )?;
    let dream_job_id = crate::db::enqueue_job(
        &conn,
        "codex-cli",
        crate::db::JobType::Dream,
        "/tmp/remem",
        None,
        "{}",
        300,
    )?;
    conn.execute(
        "UPDATE jobs SET state = 'done' WHERE id = ?1",
        [compress_job_id],
    )?;
    conn.execute(
        "UPDATE jobs
         SET state = 'failed', attempt_count = max_attempts,
             last_error = 'historical terminal dream', failure_class = 'permanent'
         WHERE id = ?1",
        [dream_job_id],
    )?;

    run_migrations(&conn)?;

    let historical: (
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<i64>,
    ) = conn.query_row(
        "SELECT followup_scheduling_state,
                followup_scheduling_completed_at_epoch,
                followup_compress_job_id,
                followup_dream_disposition,
                followup_dream_job_id
         FROM session_summaries
         WHERE memory_session_id = 'historical-rollup'",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    assert_eq!(
        historical,
        (
            Some("legacy_unknown".to_string()),
            None,
            None,
            Some("legacy_unknown".to_string()),
            None,
        )
    );

    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, session_row_id,
          covered_from_event_id, covered_to_event_id,
          followup_scheduling_state, followup_dream_disposition)
         VALUES ('new-rollup', '/tmp/remem', 8, 21, 30, NULL, NULL)",
        [],
    )?;
    let new_state: (Option<String>, Option<String>) = conn.query_row(
        "SELECT followup_scheduling_state, followup_dream_disposition
         FROM session_summaries
         WHERE memory_session_id = 'new-rollup'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(new_state, (None, None));
    let historical_jobs: (String, String, Option<String>) = conn.query_row(
        "SELECT compress.state, dream.state, dream.last_error
         FROM jobs compress, jobs dream
         WHERE compress.id = ?1 AND dream.id = ?2",
        params![compress_job_id, dream_job_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(
        historical_jobs,
        (
            "done".to_string(),
            "failed".to_string(),
            Some("historical terminal dream".to_string()),
        )
    );
    Ok(())
}

#[test]
fn v068_late_v067_rollup_defaults_legacy_unknown_and_requeues_claim() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_schema_with_pending_migrations_from(&conn, 68)?;
    let capture = crate::db::record_captured_event(
        &conn,
        &crate::db::CaptureEventInput {
            host: "codex-cli",
            session_id: "late-v067-rollup",
            project: "/tmp/remem",
            cwd: Some("/tmp/remem"),
            event_type: "session_stop",
            role: None,
            tool_name: None,
            content: r#"{"session_id":"late-v067-rollup","cwd":"/tmp/remem"}"#,
            task_kind: Some(crate::db::ExtractionTaskKind::SessionRollup),
        },
    )?;
    let task_id = capture
        .extraction_task_id
        .ok_or_else(|| anyhow::anyhow!("capture should enqueue a SessionRollup task"))?;
    let session_row_id: i64 = conn.query_row(
        "SELECT session_row_id FROM extraction_tasks WHERE id = ?1",
        [task_id],
        |row| row.get(0),
    )?;
    conn.execute(
        "UPDATE extraction_tasks
         SET status = 'processing', attempts = 4,
             lease_owner = 'worker-v067', lease_expires_epoch = 1700001000
         WHERE id = ?1",
        [task_id],
    )?;

    run_migrations(&conn)?;

    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, session_row_id,
          covered_from_event_id, covered_to_event_id)
         VALUES ('late-v067-rollup', '/tmp/remem', ?1, 1, 1)",
        [session_row_id],
    )?;
    let compress_job_id = crate::db::enqueue_job(
        &conn,
        "codex-cli",
        crate::db::JobType::Compress,
        "/tmp/remem",
        None,
        "{}",
        200,
    )?;
    let dream_job_id = crate::db::enqueue_job(
        &conn,
        "codex-cli",
        crate::db::JobType::Dream,
        "/tmp/remem",
        None,
        "{}",
        300,
    )?;
    conn.execute(
        "UPDATE jobs SET state = 'done' WHERE id = ?1",
        [compress_job_id],
    )?;
    conn.execute(
        "UPDATE jobs
         SET state = 'failed', attempt_count = max_attempts,
             last_error = 'late v067 terminal dream', failure_class = 'permanent'
         WHERE id = ?1",
        [dream_job_id],
    )?;

    let late_state: (Option<String>, Option<String>) = conn.query_row(
        "SELECT followup_scheduling_state, followup_dream_disposition
         FROM session_summaries
         WHERE memory_session_id = 'late-v067-rollup'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(
        late_state,
        (
            Some("legacy_unknown".to_string()),
            Some("legacy_unknown".to_string()),
        )
    );
    let task_state: (String, i64, Option<String>, Option<i64>) = conn.query_row(
        "SELECT status, attempts, lease_owner, lease_expires_epoch
         FROM extraction_tasks WHERE id = ?1",
        [task_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(task_state, ("pending".to_string(), 0, None, None));
    let terminal_jobs: (String, String, Option<String>) = conn.query_row(
        "SELECT compress.state, dream.state, dream.last_error
         FROM jobs compress, jobs dream
         WHERE compress.id = ?1 AND dream.id = ?2",
        params![compress_job_id, dream_job_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(
        terminal_jobs,
        (
            "done".to_string(),
            "failed".to_string(),
            Some("late v067 terminal dream".to_string()),
        )
    );
    Ok(())
}

#[test]
fn run_migrations_rejects_post_v022_schema_drift_without_repairing() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_current_schema_missing_versions(&conn, &[45])?;

    let error = run_migrations(&conn).expect_err("post-v022 drift must not be silently accepted");

    let message = format!("{error:#}");
    assert!(
        message.contains("schema drift requires manual repair"),
        "got: {message}"
    );
    assert!(
        message.contains("v045_memory_usage_columns"),
        "got: {message}"
    );
    let usage_table_exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'table' AND name = 'memory_citation_events'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(usage_table_exists, 0);
    Ok(())
}

#[test]
fn run_migrations_allows_old_v029_without_pending_v044_profile_index() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_schema_with_pending_migrations_from(&conn, 44)?;
    conn.execute_batch("DROP INDEX IF EXISTS idx_memory_embeddings_profile_memory_id;")?;

    run_migrations(&conn)?;

    assert_sqlite_object_exists(&conn, "index", "idx_memory_embeddings_profile_memory_id")?;
    assert_migration_applied(&conn, 44)?;
    Ok(())
}

#[test]
fn run_migrations_allows_old_v031_without_pending_v034_trigger_set() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    create_schema_with_pending_migrations_from(&conn, 34)?;
    for trigger in V034_GRAPH_EDGE_TRIGGERS {
        conn.execute_batch(&format!("DROP TRIGGER IF EXISTS {trigger};"))?;
    }

    run_migrations(&conn)?;

    for trigger in V034_GRAPH_EDGE_TRIGGERS {
        assert_sqlite_object_exists(&conn, "trigger", trigger)?;
    }
    assert_migration_applied(&conn, 34)?;
    Ok(())
}

const V034_GRAPH_EDGE_TRIGGERS: &[&str] = &[
    "graph_edges_validate_source_events_insert",
    "graph_edges_validate_source_events_update",
    "graph_edges_validate_nodes_insert",
    "graph_edges_validate_nodes_update",
    "graph_edges_memories_delete",
    "graph_edges_entities_delete",
    "graph_edges_memory_facts_delete",
    "graph_edges_captured_events_delete",
    "graph_edges_topic_segments_delete",
    "graph_edges_memory_state_keys_delete",
    "graph_edges_graph_file_nodes_delete",
];

fn create_current_schema_missing_versions(
    conn: &Connection,
    missing_versions: &[i64],
) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=OFF;
         PRAGMA writable_schema=ON;",
    )?;
    for migration in MIGRATIONS
        .iter()
        .filter(|migration| !missing_versions.contains(&migration.version))
    {
        conn.execute_batch(migration.sql)?;
    }
    conn.execute_batch(
        "CREATE TABLE _schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at_epoch INTEGER NOT NULL
        );",
    )?;
    for migration in MIGRATIONS {
        conn.execute(
            "INSERT INTO _schema_migrations (version, name, applied_at_epoch)
             VALUES (?1, ?2, 1700000000)",
            params![migration.version, migration.name],
        )?;
    }
    conn.execute_batch(&format!(
        "PRAGMA writable_schema=OFF;
         PRAGMA user_version = {};
         PRAGMA foreign_keys=ON;",
        super::types::OLD_BASELINE_VERSION - 1 + super::latest_schema_version()
    ))?;
    Ok(())
}

fn create_schema_with_pending_migrations_from(
    conn: &Connection,
    pending_from_version: i64,
) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=OFF;")?;
    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version < pending_from_version)
    {
        conn.execute_batch(migration.sql)?;
    }
    conn.execute_batch(
        "CREATE TABLE _schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at_epoch INTEGER NOT NULL
        );",
    )?;
    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version < pending_from_version)
    {
        conn.execute(
            "INSERT INTO _schema_migrations (version, name, applied_at_epoch)
             VALUES (?1, ?2, 1700000000)",
            params![migration.version, migration.name],
        )?;
    }
    conn.execute_batch(&format!(
        "PRAGMA user_version = {}; PRAGMA foreign_keys=ON;",
        super::types::OLD_BASELINE_VERSION + pending_from_version - 2
    ))?;
    Ok(())
}

fn assert_sqlite_object_exists(
    conn: &Connection,
    object_type: &str,
    object_name: &str,
) -> Result<()> {
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type=?1 AND name=?2",
            params![object_type, object_name],
            |_| Ok(true),
        )
        .unwrap_or(false);
    assert!(exists, "{object_type} {object_name} should exist");
    Ok(())
}

fn assert_migration_applied(conn: &Connection, version: i64) -> Result<()> {
    let applied: bool = conn
        .query_row(
            "SELECT 1 FROM _schema_migrations WHERE version=?1",
            [version],
            |_| Ok(true),
        )
        .unwrap_or(false);
    assert!(applied, "migration v{version:03} should be marked applied");
    Ok(())
}
