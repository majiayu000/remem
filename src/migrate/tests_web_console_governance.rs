use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::db::test_support::ScopedTestDataDir;

use crate::migrate::run::{run_post_migration_hook, run_pre_migration_hook};
use crate::migrate::state::{ensure_migration_table, mark_applied};
use crate::migrate::{run_migrations, validate_schema_invariants, MIGRATIONS};

const V070: i64 = 70;

fn pre_v070(label: &str) -> Result<(ScopedTestDataDir, Connection)> {
    let data_dir = ScopedTestDataDir::new(label);
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys = ON")?;
    ensure_migration_table(&conn)?;
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| {
        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version < V070)
        {
            run_pre_migration_hook(&conn, migration.version, migration.name)?;
            conn.execute_batch(migration.sql).with_context(|| {
                format!(
                    "apply pre-v070 migration v{:03}_{}",
                    migration.version, migration.name
                )
            })?;
            run_post_migration_hook(&conn, migration.version, migration.name)?;
            mark_applied(&conn, migration.version, migration.name)?;
        }
        Ok::<_, anyhow::Error>(())
    })();
    match result {
        Ok(()) => conn.execute_batch("COMMIT")?,
        Err(error) => {
            conn.execute_batch("ROLLBACK")?;
            return Err(error);
        }
    }
    Ok((data_dir, conn))
}

#[derive(Debug)]
struct CursorFixture {
    host_id: i64,
    workspace_id: i64,
    project_id: i64,
}

fn insert_cursor_fixture(conn: &Connection) -> Result<CursorFixture> {
    let now = 1_700_000_000_i64;
    let host_id = conn.query_row("SELECT id FROM hosts ORDER BY id LIMIT 1", [], |row| {
        row.get(0)
    })?;
    conn.execute(
        "INSERT INTO workspaces(id, root_path, created_at_epoch, updated_at_epoch)
         VALUES (11, '/tmp/gh880-v070', ?1, ?1)",
        [now],
    )?;
    conn.execute(
        "INSERT INTO projects(id, workspace_id, project_path, project_key,
                              created_at_epoch, updated_at_epoch)
         VALUES (12, 11, '/tmp/gh880-v070', 'gh880-v070', ?1, ?1)",
        [now],
    )?;
    for id in [101_i64, 102] {
        conn.execute(
            "INSERT INTO sessions(id, host_id, workspace_id, project_id, session_id,
                                  started_at_epoch, last_seen_at_epoch, status)
             VALUES (?1, ?2, 11, 12, ?3, ?4, ?4, 'active')",
            params![id, host_id, format!("session-{id}"), now + id],
        )?;
    }
    for id in [201_i64, 202] {
        conn.execute(
            "INSERT INTO captured_events(
                 id, host_id, workspace_id, project_id, session_row_id, session_id,
                 event_id, event_type, content_hash, retention_class,
                 created_at_epoch, inserted_at_epoch
             ) VALUES (?1, ?2, 11, 12, 101, 'session-101', ?3, 'message',
                       ?4, 'default', ?5, ?5)",
            params![
                id,
                host_id,
                format!("event-{id}"),
                format!("hash-{id}"),
                now + id
            ],
        )?;
    }
    for id in [301_i64, 302] {
        conn.execute(
            "INSERT INTO extraction_tasks(
                 id, task_kind, host_id, workspace_id, project_id, session_row_id,
                 priority, status, idempotency_key, attempts, created_at_epoch,
                 updated_at_epoch
             ) VALUES (?1, 'observation_extract', ?2, 11, 12, 101, 100,
                       'pending', ?3, 0, ?4, ?4)",
            params![id, host_id, format!("task-{id}"), now + id],
        )?;
    }
    for id in [401_i64, 402] {
        conn.execute(
            "INSERT INTO observations(
                 id, memory_session_id, project, type, title, created_at_epoch, status,
                 host_id, project_id, session_row_id, observation_type,
                 evidence_event_ids, confidence
             ) VALUES (?1, 'session-101', '/tmp/gh880-v070', 'discovery', ?2,
                       ?3, 'active', ?4, 12, 101, 'discovery', '[201]', 0.9)",
            params![id, format!("observation-{id}"), now + id, host_id],
        )?;
    }
    conn.execute(
        "INSERT INTO workstreams(
             id, project, title, status, created_at_epoch, updated_at_epoch,
             target_project, owner_scope, owner_key, identity_key
         ) VALUES (501, '/tmp/gh880-v070', 'root', 'active', ?1, ?1,
                   '/tmp/gh880-v070', 'repo', '/tmp/gh880-v070', 'root')",
        [now],
    )?;
    conn.execute(
        "INSERT INTO workstreams(
             id, project, title, status, created_at_epoch, updated_at_epoch,
             target_project, owner_scope, owner_key, identity_key,
             merged_into_workstream_id
         ) VALUES (502, '/tmp/gh880-v070', 'alias', 'merged', ?1, ?1,
                   '/tmp/gh880-v070', 'repo', '/tmp/gh880-v070', 'alias', 501)",
        [now],
    )?;
    Ok(CursorFixture {
        host_id,
        workspace_id: 11,
        project_id: 12,
    })
}

fn foreign_keys_enabled(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))? != 0)
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(columns.iter().any(|candidate| candidate == column))
}

fn update_trigger_columns(conn: &Connection, trigger: &str) -> Result<Vec<String>> {
    let sql: String = conn.query_row(
        "SELECT sql FROM sqlite_schema WHERE type = 'trigger' AND name = ?1",
        [trigger],
        |row| row.get(0),
    )?;
    let normalized = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    let (_, columns_and_rest) = normalized
        .split_once("AFTER UPDATE OF")
        .with_context(|| format!("{trigger} must declare AFTER UPDATE OF columns"))?;
    let (columns, _) = columns_and_rest
        .split_once(" ON ")
        .with_context(|| format!("{trigger} must declare its target table"))?;
    let mut parsed = columns
        .split(',')
        .map(|column| column.trim().to_string())
        .collect::<Vec<_>>();
    parsed.sort();
    Ok(parsed)
}

#[test]
fn v070_migration_remains_named_stably() -> Result<()> {
    let migration = MIGRATIONS
        .iter()
        .find(|migration| migration.version == V070)
        .context("v070 migration is missing")?;
    assert_eq!(migration.version, V070);
    assert_eq!(migration.name, "web_console_governance");
    Ok(())
}

#[test]
fn v070_upgrade_preserves_rows_objects_fts_and_enables_foreign_keys() -> Result<()> {
    let (_data_dir, conn) = pre_v070("v070-upgrade")?;
    let fixture = insert_cursor_fixture(&conn)?;
    conn.execute_batch("PRAGMA foreign_keys = OFF")?;

    run_migrations(&conn)?;

    assert!(foreign_keys_enabled(&conn)?);
    for table in [
        "observations",
        "sessions",
        "workstreams",
        "captured_events",
        "extraction_tasks",
    ] {
        let sql: String = conn.query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        )?;
        assert!(sql.contains("PRIMARY KEY AUTOINCREMENT"), "{table}: {sql}");
    }
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM observations WHERE id IN (401, 402)",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        2
    );
    assert_eq!(
        conn.query_row(
            "SELECT merged_into_workstream_id FROM workstreams WHERE id = 502",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        501
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM observations_fts WHERE rowid = 402",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        1
    );
    for trigger in [
        "observations_ai",
        "observations_ad",
        "observations_au",
        "graph_edges_captured_events_delete",
        "graph_edges_validate_source_events_insert",
        "graph_edges_validate_source_events_update",
        "graph_edges_validate_nodes_insert",
        "graph_edges_validate_nodes_update",
    ] {
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'trigger' AND name = ?1",
                [trigger],
                |row| row.get::<_, i64>(0)
            )?,
            1,
            "missing trigger {trigger}"
        );
    }
    assert!(validate_schema_invariants(&conn)?.is_empty());
    assert_eq!(fixture.workspace_id, 11);
    assert_eq!(fixture.project_id, 12);
    Ok(())
}

#[test]
fn v070_autoincrement_never_reuses_deleted_migration_maxima() -> Result<()> {
    let (_data_dir, conn) = pre_v070("v070-autoincrement")?;
    let fixture = insert_cursor_fixture(&conn)?;
    run_migrations(&conn)?;
    let now = 1_800_000_000_i64;

    conn.execute("DELETE FROM observations WHERE id = 402", [])?;
    conn.execute(
        "INSERT INTO observations(memory_session_id, type, title) VALUES ('new', 'discovery', 'new')",
        [],
    )?;
    assert!(conn.last_insert_rowid() > 402);

    conn.execute("DELETE FROM sessions WHERE id = 102", [])?;
    conn.execute(
        "INSERT INTO sessions(host_id, workspace_id, project_id, session_id,
                              last_seen_at_epoch, status)
         VALUES (?1, ?2, ?3, 'session-new', ?4, 'active')",
        params![
            fixture.host_id,
            fixture.workspace_id,
            fixture.project_id,
            now
        ],
    )?;
    assert!(conn.last_insert_rowid() > 102);

    conn.execute("DELETE FROM workstreams WHERE id = 502", [])?;
    conn.execute(
        "INSERT INTO workstreams(project, title, created_at_epoch, updated_at_epoch)
         VALUES ('/tmp/gh880-v070', 'new', ?1, ?1)",
        [now],
    )?;
    assert!(conn.last_insert_rowid() > 502);

    conn.execute("DELETE FROM captured_events WHERE id = 202", [])?;
    conn.execute(
        "INSERT INTO captured_events(
             host_id, workspace_id, project_id, session_row_id, session_id,
             event_id, event_type, content_hash, retention_class,
             created_at_epoch, inserted_at_epoch
         ) VALUES (?1, ?2, ?3, 101, 'session-101', 'event-new', 'message',
                   'hash-new', 'default', ?4, ?4)",
        params![
            fixture.host_id,
            fixture.workspace_id,
            fixture.project_id,
            now
        ],
    )?;
    assert!(conn.last_insert_rowid() > 202);

    conn.execute("DELETE FROM extraction_tasks WHERE id = 302", [])?;
    conn.execute(
        "INSERT INTO extraction_tasks(
             task_kind, host_id, workspace_id, project_id, session_row_id,
             priority, status, idempotency_key, created_at_epoch, updated_at_epoch
         ) VALUES ('observation_extract', ?1, ?2, ?3, 101, 100, 'pending',
                   'task-new', ?4, ?4)",
        params![
            fixture.host_id,
            fixture.workspace_id,
            fixture.project_id,
            now
        ],
    )?;
    assert!(conn.last_insert_rowid() > 302);
    Ok(())
}

#[test]
fn v070_versions_all_visible_updates_and_clears_archive_marker() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    run_migrations(&conn)?;
    conn.execute(
        "INSERT INTO memory_candidates(
             scope, memory_type, topic_key, text, evidence_event_ids, confidence,
             risk_class, review_status, created_at_epoch, updated_at_epoch
         ) VALUES ('project', 'decision', 'v070', 'before', '[]', 0.9,
                   'low', 'pending_review', 1, 1)",
        [],
    )?;
    let candidate_id = conn.last_insert_rowid();
    conn.execute(
        "UPDATE memory_candidates SET text = 'after', review_status = 'accepted'
         WHERE id = ?1",
        [candidate_id],
    )?;
    assert_eq!(
        conn.query_row(
            "SELECT version FROM memory_candidates WHERE id = ?1",
            [candidate_id],
            |row| row.get::<_, i64>(0)
        )?,
        2
    );

    conn.execute(
        "INSERT INTO memories(project, title, content, memory_type,
                              created_at_epoch, updated_at_epoch, status,
                              web_archive_operation_id)
         VALUES ('p', 'before', 'content', 'decision', 1, 1, 'active', 'op_old')",
        [],
    )?;
    let memory_id = conn.last_insert_rowid();
    conn.execute(
        "UPDATE memories SET status = 'archived', updated_at_epoch = 2 WHERE id = ?1",
        [memory_id],
    )?;
    let (version, marker): (i64, Option<String>) = conn.query_row(
        "SELECT version, web_archive_operation_id FROM memories WHERE id = ?1",
        [memory_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(version, 2);
    assert_eq!(marker, None);
    conn.execute(
        "UPDATE memories SET access_count = access_count + 1 WHERE id = ?1",
        [memory_id],
    )?;
    assert_eq!(
        conn.query_row(
            "SELECT version FROM memories WHERE id = ?1",
            [memory_id],
            |row| row.get::<_, i64>(0)
        )?,
        2
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'before'",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        1,
        "version-only and status updates must not corrupt memories FTS"
    );
    Ok(())
}

#[test]
fn v070_version_triggers_match_web_visible_column_allowlists() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    run_migrations(&conn)?;
    let mut candidate_columns = [
        "project_id",
        "scope",
        "memory_type",
        "topic_key",
        "text",
        "evidence_event_ids",
        "confidence",
        "risk_class",
        "review_status",
        "updated_at_epoch",
        "source_project",
        "target_project",
        "owner_scope",
        "owner_key",
        "topic_domain",
        "routing_confidence",
        "routing_reason",
        "context_class",
        "expires_at_epoch",
        "valid_from_epoch",
        "valid_to_epoch",
        "state_key",
        "state_key_confidence",
        "state_key_reason",
        "auto_promote_block_reason",
        "source_kind",
        "review_actor",
        "reviewed_at_epoch",
        "review_action_source",
        "review_batch_id",
        "review_reason",
        "source_trust_class",
        "quarantine_pattern_id",
        "quarantine_pattern_version",
        "acknowledged_pattern_id",
        "acknowledged_pattern_version",
        "acknowledged_at_epoch",
    ]
    .map(str::to_string)
    .to_vec();
    candidate_columns.sort();
    assert_eq!(
        update_trigger_columns(&conn, "memory_candidates_web_version")?,
        candidate_columns
    );

    let mut memory_columns = [
        "session_id",
        "project",
        "topic_key",
        "title",
        "content",
        "memory_type",
        "files",
        "updated_at_epoch",
        "status",
        "branch",
        "scope",
        "evidence_event_ids",
        "source_candidate_id",
        "confidence",
        "search_context",
        "source_project",
        "target_project",
        "owner_scope",
        "owner_key",
        "topic_domain",
        "routing_confidence",
        "routing_reason",
        "context_class",
        "expires_at_epoch",
        "valid_from_epoch",
        "valid_to_epoch",
        "state_key_id",
        "reference_time_epoch",
        "source_trust_class",
        "acknowledged_pattern_id",
        "acknowledged_pattern_version",
        "acknowledged_at_epoch",
    ]
    .map(str::to_string)
    .to_vec();
    memory_columns.sort();
    assert_eq!(
        update_trigger_columns(&conn, "memories_web_version")?,
        memory_columns
    );
    Ok(())
}

#[test]
fn v070_migration_failure_rolls_back_schema_and_restores_foreign_keys() -> Result<()> {
    let (_data_dir, conn) = pre_v070("v070-migration-failure")?;
    conn.execute_batch("CREATE TABLE api_mutation_requests(unexpected TEXT)")?;
    let error = run_migrations(&conn).expect_err("conflicting ledger table must fail v070");
    assert!(format!("{error:#}").contains("api_mutation_requests"));
    assert!(foreign_keys_enabled(&conn)?);
    assert!(!has_column(&conn, "memories", "version")?);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM _schema_migrations WHERE version = 70",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        0
    );
    Ok(())
}

#[test]
fn v070_integrity_failure_rolls_back_rebuild_and_restores_foreign_keys() -> Result<()> {
    let (_data_dir, conn) = pre_v070("v070-integrity-failure")?;
    conn.execute_batch("PRAGMA foreign_keys = OFF")?;
    conn.execute(
        "INSERT INTO sessions(id, host_id, workspace_id, project_id, session_id,
                              last_seen_at_epoch, status)
         VALUES (999, 999, 999, 999, 'orphan', 1, 'active')",
        [],
    )?;
    let error = run_migrations(&conn).expect_err("foreign key check must reject orphan row");
    assert!(error.to_string().contains("foreign key check failed"));
    assert!(foreign_keys_enabled(&conn)?);
    assert!(!has_column(&conn, "memories", "version")?);
    Ok(())
}

#[test]
fn v070_schema_drift_reports_missing_new_objects() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    run_migrations(&conn)?;
    conn.execute_batch("DROP TRIGGER memories_web_version")?;
    let errors = validate_schema_invariants(&conn)?;
    assert!(errors.iter().any(|error| {
        error.contains("v070_web_console_governance")
            && error.contains("trigger memories_web_version")
    }));
    Ok(())
}
