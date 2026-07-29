use anyhow::Result;
use rusqlite::{params, Connection};

use super::{run_migrations, validate_schema_invariants, MIGRATIONS};

fn setup_v073() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    for migration in MIGRATIONS.iter().filter(|migration| migration.version < 74) {
        conn.execute_batch(migration.sql)?;
    }
    Ok(conn)
}

fn apply_v074(conn: &Connection) -> Result<()> {
    let migration = MIGRATIONS
        .iter()
        .find(|migration| migration.version == 74)
        .expect("v074 migration must be registered");
    conn.execute_batch(migration.sql)?;
    Ok(())
}

fn insert_event(conn: &Connection, event_type: &str, created_at_epoch: i64) -> Result<i64> {
    conn.execute(
        "INSERT INTO events
         (session_id, project, event_type, summary, created_at_epoch)
         VALUES ('session', '/repo', ?1, 'summary', ?2)",
        params![event_type, created_at_epoch],
    )?;
    Ok(conn.last_insert_rowid())
}

fn insert_raw_job(
    conn: &Connection,
    host: &str,
    job_type: &str,
    project: &str,
    session_id: Option<&str>,
    state: &str,
    now_epoch: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO jobs
         (host, job_type, project, session_id, payload_json, state, priority,
          attempt_count, max_attempts, next_retry_epoch, created_at_epoch,
          updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, '{}', ?5, 100, 0, 6, 0, ?6, ?6)",
        params![host, job_type, project, session_id, state, now_epoch],
    )?;
    Ok(conn.last_insert_rowid())
}

#[test]
fn v074_backfills_only_known_ephemeral_events_and_defaults_unknown_to_audit() -> Result<()> {
    let conn = setup_v073()?;
    let ephemeral_types = [
        "file_edit",
        "file_create",
        "bash",
        "search",
        "agent",
        "tool_result",
        "cursor_tool_failure",
    ];
    for event_type in ephemeral_types {
        insert_event(&conn, event_type, 100)?;
    }
    for event_type in ["memory_governance", "scope_cleanup", "future_event"] {
        insert_event(&conn, event_type, 100)?;
    }

    apply_v074(&conn)?;

    for event_type in ephemeral_types {
        let retention: String = conn.query_row(
            "SELECT retention_class FROM events WHERE event_type = ?1",
            [event_type],
            |row| row.get(0),
        )?;
        assert_eq!(retention, "ephemeral", "event_type={event_type}");
    }
    for event_type in ["memory_governance", "scope_cleanup", "future_event"] {
        let retention: String = conn.query_row(
            "SELECT retention_class FROM events WHERE event_type = ?1",
            [event_type],
            |row| row.get(0),
        )?;
        assert_eq!(retention, "audit", "event_type={event_type}");
    }

    let new_id = insert_event(&conn, "file_edit", 200)?;
    let new_retention: String = conn.query_row(
        "SELECT retention_class FROM events WHERE id = ?1",
        [new_id],
        |row| row.get(0),
    )?;
    assert_eq!(new_retention, "audit");
    Ok(())
}

#[test]
fn v074_prevents_deleting_api_mutation_audit_provenance() -> Result<()> {
    let conn = setup_v073()?;
    let audit_id = insert_event(&conn, "memory_governance", 100)?;
    let disposable_id = insert_event(&conn, "bash", 100)?;
    conn.execute(
        "INSERT INTO api_mutation_requests
         (idempotency_key_hash, request_hash, operation_id, resource_kind,
          resource_id, action, response_schema_version, response_json, audit_id,
          created_at_epoch)
         VALUES ('idem', 'request', 'operation', 'memory', 1, 'archive', 1,
                 '{}', ?1, 100)",
        [audit_id],
    )?;

    apply_v074(&conn)?;

    let audit_index_columns = conn
        .prepare("SELECT name FROM pragma_index_info('idx_api_mutation_requests_audit')")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(audit_index_columns, vec!["audit_id"]);
    let cleanup_plan = conn
        .prepare(
            "EXPLAIN QUERY PLAN
             SELECT id FROM events
             WHERE retention_class = 'ephemeral'
               AND created_at_epoch < 200
               AND NOT EXISTS (
                 SELECT 1 FROM api_mutation_requests request
                 WHERE request.audit_id = events.id
               )",
        )?
        .query_map([], |row| row.get::<_, String>(3))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert!(
        cleanup_plan.iter().any(|detail| {
            detail.contains("idx_api_mutation_requests_audit")
                && !detail.starts_with("SCAN request")
        }),
        "audit anti-join must use its index: {cleanup_plan:?}"
    );

    let error = conn
        .execute("DELETE FROM events WHERE id = ?1", [audit_id])
        .expect_err("referenced audit event deletion must fail");
    assert!(
        error
            .to_string()
            .contains("cannot delete event referenced by api_mutation_requests.audit_id"),
        "got: {error}"
    );
    assert_eq!(
        conn.execute("DELETE FROM events WHERE id = ?1", [disposable_id])?,
        1
    );
    Ok(())
}

#[test]
fn v074_enforces_global_cleanup_identity_and_ledger_shape() -> Result<()> {
    let conn = setup_v073()?;
    apply_v074(&conn)?;
    let first = insert_raw_job(
        &conn,
        "worker-a",
        "cleanup",
        "/project-a",
        Some("session-a"),
        "pending",
        100,
    )?;
    let duplicate = insert_raw_job(
        &conn,
        "worker-b",
        "cleanup",
        "/project-b",
        Some("session-b"),
        "processing",
        101,
    );
    assert!(duplicate.is_err(), "Cleanup active identity must be global");

    conn.execute("UPDATE jobs SET state = 'done' WHERE id = ?1", [first])?;
    let second = insert_raw_job(
        &conn,
        "worker-b",
        "cleanup",
        "/project-b",
        Some("session-b"),
        "pending",
        102,
    )?;

    conn.execute(
        "INSERT INTO maintenance_runs
         (job_id, \"trigger\", policy_version, started_at_epoch,
          finished_at_epoch, outcome, counts_json, error)
         VALUES (?1, 'automatic', 1, 100, 101, 'success', '{}', NULL)",
        [second],
    )?;
    assert!(
        conn.execute(
            "INSERT INTO maintenance_runs
             (job_id, \"trigger\", policy_version, started_at_epoch,
              finished_at_epoch, outcome, counts_json, error)
             VALUES (NULL, 'automatic', 1, 100, 101, 'success', NULL, NULL)",
            [],
        )
        .is_err(),
        "successful runs require JSON counts"
    );
    assert!(
        conn.execute(
            "INSERT INTO maintenance_runs
             (job_id, \"trigger\", policy_version, started_at_epoch,
              finished_at_epoch, outcome, counts_json, error)
             VALUES (NULL, 'automatic', 1, 100, 101, 'failure', '{}', 'boom')",
            [],
        )
        .is_err(),
        "failed runs must not claim success counts"
    );

    let ordinary_index_sql: String = conn.query_row(
        "SELECT sql FROM sqlite_master
         WHERE type = 'index' AND name = 'idx_jobs_active_ordinary_unique'",
        [],
        |row| row.get(0),
    )?;
    assert!(
        ordinary_index_sql.contains("'cleanup'"),
        "ordinary identity must exclude cleanup: {ordinary_index_sql}"
    );
    Ok(())
}

#[test]
fn v074_schema_drift_is_reported() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    run_migrations(&conn)?;
    assert!(validate_schema_invariants(&conn)?.is_empty());

    conn.execute_batch(
        "DROP TRIGGER events_preserve_api_mutation_audit;
         DROP INDEX idx_api_mutation_requests_audit;",
    )?;
    let errors = validate_schema_invariants(&conn)?;
    assert!(
        errors
            .iter()
            .any(|error| error.contains("v074_automatic_cleanup")
                && error.contains("events_preserve_api_mutation_audit")),
        "got: {errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("v074_automatic_cleanup")
                && error.contains("idx_api_mutation_requests_audit")),
        "got: {errors:?}"
    );
    Ok(())
}
