use anyhow::Result;
use rusqlite::{params, Connection};

use super::{run_migrations, MIGRATIONS};

struct GraphSchemaFixture {
    now: i64,
    project_id: i64,
    episode_id: i64,
    memory_id: i64,
    other_memory_id: i64,
    candidate_id: i64,
    operation_id: i64,
}

fn setup_graph_schema_fixture(conn: &Connection) -> Result<GraphSchemaFixture> {
    let now = 1_700_000_000_i64;
    let host_id: i64 =
        conn.query_row("SELECT id FROM hosts WHERE name = 'codex-cli'", [], |row| {
            row.get(0)
        })?;
    conn.execute(
        "INSERT INTO workspaces(root_path, git_remote, git_branch, created_at_epoch, updated_at_epoch)
         VALUES ('/tmp/remem-graph-schema', 'origin', 'main', ?1, ?1)",
        [now],
    )?;
    let workspace_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO projects(workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES (?1, '/tmp/remem-graph-schema', 'tmp-remem-graph-schema', ?2, ?2)",
        params![workspace_id, now],
    )?;
    let project_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO sessions(host_id, workspace_id, project_id, session_id, started_at_epoch,
                              last_seen_at_epoch, status)
         VALUES (?1, ?2, ?3, 'session-a', ?4, ?4, 'active')",
        params![host_id, workspace_id, project_id, now],
    )?;
    let session_row_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO captured_events(host_id, workspace_id, project_id, session_row_id,
                                     session_id, event_id, event_type, content_hash,
                                     retention_class, created_at_epoch, inserted_at_epoch)
         VALUES (?1, ?2, ?3, ?4, 'session-a', 'event-a', 'message',
                 'hash-a', 'default', ?5, ?5)",
        params![host_id, workspace_id, project_id, session_row_id, now],
    )?;
    let episode_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO memories(project, topic_key, title, content, memory_type,
                              created_at_epoch, updated_at_epoch, status)
         VALUES ('/tmp/remem-graph-schema', 'graph-schema', 'Graph schema',
                 'Schema rejects graph self-edges.', 'decision', ?1, ?1, 'active')",
        [now],
    )?;
    let memory_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO memories(project, topic_key, title, content, memory_type,
                              created_at_epoch, updated_at_epoch, status)
         VALUES ('/tmp/remem-graph-schema', 'graph-schema-2', 'Graph schema 2',
                 'Schema rejects dangling candidate provenance.', 'decision', ?1, ?1, 'active')",
        [now],
    )?;
    let other_memory_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO memory_candidates(project_id, scope, memory_type, topic_key, text,
                                       evidence_event_ids, confidence, risk_class,
                                       review_status, created_at_epoch, updated_at_epoch)
         VALUES (?1, 'project', 'decision', 'graph-schema', 'Graph schema.',
                 ?2, 0.9, 'low', 'accepted', ?3, ?3)",
        params![project_id, format!("[{episode_id}]"), now],
    )?;
    let candidate_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO memory_operation_log(operation, planner_version, actor, source,
                                         source_candidate_id, result_memory_id,
                                         confidence, reason, created_at_epoch)
         VALUES ('add', 'graph-schema-test', 'test', 'memory_candidate',
                 ?1, ?2, 0.9, 'test provenance', ?3)",
        params![candidate_id, memory_id, now],
    )?;
    let operation_id = conn.last_insert_rowid();

    Ok(GraphSchemaFixture {
        now,
        project_id,
        episode_id,
        memory_id,
        other_memory_id,
        candidate_id,
        operation_id,
    })
}

#[test]
fn memory_ownership_migration_backfills_legacy_rows() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

    for migration in MIGRATIONS.iter().filter(|migration| migration.version < 19) {
        conn.execute_batch(migration.sql)?;
    }
    conn.execute_batch(
        "CREATE TABLE _schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at_epoch INTEGER NOT NULL
        );",
    )?;
    for migration in MIGRATIONS.iter().filter(|migration| migration.version < 19) {
        conn.execute(
            "INSERT INTO _schema_migrations (version, name, applied_at_epoch)
             VALUES (?1, ?2, ?3)",
            params![migration.version, migration.name, 1_700_000_000_i64],
        )?;
    }

    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope)
         VALUES (1, 's1', '/repo', 'repo-fact', 'Repo fact', 'body', 'decision',
          NULL, 100, 100, 'active', NULL, 'project')",
        [],
    )?;
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope)
         VALUES (2, 's1', '/repo', 'global-pref', 'Global pref', 'body', 'preference',
          NULL, 100, 100, 'active', NULL, 'global')",
        [],
    )?;
    conn.execute(
        "INSERT INTO workstreams
         (id, project, title, description, status, progress, next_action, blockers,
          created_at_epoch, updated_at_epoch, completed_at_epoch)
         VALUES (1, '/repo', 'Ship feature', NULL, 'active', NULL, NULL, NULL,
          100, 100, NULL)",
        [],
    )?;
    conn.execute(
        "INSERT INTO workspaces(id, root_path, git_remote, git_branch, created_at_epoch, updated_at_epoch)
         VALUES (1, '/repo', NULL, NULL, 100, 100)",
        [],
    )?;
    conn.execute(
        "INSERT INTO projects(id, workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES (1, 1, '/repo', 'repo', 100, 100)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_candidates
         (id, project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (1, 1, 'project', 'decision', 'candidate-fact', 'body', '[1]',
          0.9, 'low', 'pending_review', 100, 100)",
        [],
    )?;
    conn.execute(
        "INSERT INTO session_summaries
         (id, memory_session_id, project, request, completed, decisions, learned,
          next_steps, preferences, prompt_number, created_at, created_at_epoch,
          discovery_tokens, project_id)
         VALUES (1, 'm1', '/legacy-repo', 'req', 'done', '[]', '[]',
          '[]', '[]', 1, 'now', 100, 0, 1)",
        [],
    )?;

    run_migrations(&conn)?;

    let memory: (String, Option<String>, String, String, String) = conn.query_row(
        "SELECT source_project, target_project, owner_scope, owner_key, context_class
         FROM memories WHERE id = 1",
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
        memory,
        (
            "/repo".to_string(),
            Some("/repo".to_string()),
            "repo".to_string(),
            "/repo".to_string(),
            "startup_core".to_string()
        )
    );

    let global: (String, Option<String>, String, String) = conn.query_row(
        "SELECT source_project, target_project, owner_scope, owner_key
         FROM memories WHERE id = 2",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(
        global,
        (
            "/repo".to_string(),
            None,
            "user".to_string(),
            "user:default".to_string()
        )
    );

    let candidate: (String, Option<String>, String, String) = conn.query_row(
        "SELECT source_project, target_project, owner_scope, owner_key
         FROM memory_candidates WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(
        candidate,
        (
            "/repo".to_string(),
            Some("/repo".to_string()),
            "repo".to_string(),
            "/repo".to_string()
        )
    );

    let workstream: (String, String, String, String) = conn.query_row(
        "SELECT source_project, target_project, owner_scope, owner_key
         FROM workstreams WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(
        workstream,
        (
            "/repo".to_string(),
            "/repo".to_string(),
            "repo".to_string(),
            "/repo".to_string()
        )
    );

    let summary: (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    ) = conn.query_row(
        "SELECT source_project, target_project, owner_scope, owner_key, context_class
         FROM session_summaries WHERE id = 1",
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
        summary,
        (
            "/repo".to_string(),
            None,
            None,
            None,
            "search_only".to_string()
        )
    );

    Ok(())
}

/// After v020 (#236) memories_fts indexes EVERY row regardless of status: the
/// index-layer `WHERE status='active'` filter is gone, and visibility is now
/// enforced only by the post-JOIN `m.status` predicate in fts.rs.
#[test]
fn memories_fts_indexes_all_statuses_after_v020() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    run_migrations(&conn)?;

    conn.execute(
        "INSERT INTO memories(id, project, title, content, memory_type, created_at_epoch,
            updated_at_epoch, status)
         VALUES (1, 'proj', 'stale title', 'stale body needle', 'discovery', 100, 100, 'stale')",
        [],
    )?;
    let count_match = |needle: &str| -> Result<i64> {
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH ?1",
            [needle],
            |row| row.get(0),
        )?)
    };
    assert_eq!(
        count_match("needle")?,
        1,
        "stale rows must enter memories_fts"
    );

    conn.execute(
        "UPDATE memories SET status = 'active', updated_at_epoch = 200 WHERE id = 1",
        [],
    )?;
    assert_eq!(count_match("needle")?, 1);
    conn.execute(
        "UPDATE memories SET status = 'archived', updated_at_epoch = 300 WHERE id = 1",
        [],
    )?;
    assert_eq!(
        count_match("needle")?,
        1,
        "archived rows stay indexed; visibility is a query-layer concern now"
    );

    conn.execute("DELETE FROM memories WHERE id = 1", [])?;
    assert_eq!(count_match("needle")?, 0, "delete must remove the FTS row");

    Ok(())
}

#[test]
fn graph_edges_reject_self_edges_at_schema_boundary() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;
    let fixture = setup_graph_schema_fixture(&conn)?;

    let err = conn
        .execute(
            "INSERT INTO graph_edges
             (edge_type, edge_trust, from_node_kind, from_node_id, to_node_kind, to_node_id,
              source_event_ids, source_candidate_id, source_operation_id, confidence, reason,
             created_at_epoch)
             VALUES ('duplicates', 'trusted', 'memory', ?1, 'memory', ?1,
                     ?2, ?3, ?4, 0.9, 'self edge', ?5)",
            params![
                fixture.memory_id,
                format!("[{}]", fixture.episode_id),
                fixture.candidate_id,
                fixture.operation_id,
                fixture.now
            ],
        )
        .expect_err("raw SQL graph self-edge must fail");
    assert!(err.to_string().contains("CHECK constraint failed"));

    Ok(())
}

#[test]
fn graph_edges_reject_dangling_source_candidate_at_schema_boundary() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;
    let fixture = setup_graph_schema_fixture(&conn)?;

    let err = conn
        .execute(
            "INSERT INTO graph_edges
             (edge_type, edge_trust, from_node_kind, from_node_id, to_node_kind, to_node_id,
              source_event_ids, source_candidate_id, source_operation_id, confidence, reason,
              created_at_epoch)
             VALUES ('duplicates', 'trusted', 'memory', ?1, 'memory', ?2,
                     ?3, ?4, ?5, 0.9, 'dangling candidate', ?6)",
            params![
                fixture.memory_id,
                fixture.other_memory_id,
                format!("[{}]", fixture.episode_id),
                fixture.candidate_id + 10_000,
                fixture.operation_id,
                fixture.now
            ],
        )
        .expect_err("raw SQL graph source_candidate_id must reference an existing candidate");
    assert!(err
        .to_string()
        .contains("graph_edges source candidate missing"));

    conn.execute(
        "INSERT INTO graph_edges
         (edge_type, edge_trust, from_node_kind, from_node_id, to_node_kind, to_node_id,
          source_event_ids, source_candidate_id, source_operation_id, confidence, reason,
          created_at_epoch)
         VALUES ('duplicates', 'trusted', 'memory', ?1, 'memory', ?2,
                 ?3, ?4, ?5, 0.9, 'valid candidate', ?6)",
        params![
            fixture.memory_id,
            fixture.other_memory_id,
            format!("[{}]", fixture.episode_id),
            fixture.candidate_id,
            fixture.operation_id,
            fixture.now
        ],
    )?;
    let edge_id = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO memory_candidates(project_id, scope, memory_type, topic_key, text,
                                       evidence_event_ids, confidence, risk_class,
                                       review_status, created_at_epoch, updated_at_epoch)
         VALUES (?1, 'project', 'decision', 'graph-schema-other', 'Other graph schema.',
                 ?2, 0.9, 'low', 'accepted', ?3, ?3)",
        params![
            fixture.project_id,
            format!("[{}]", fixture.episode_id),
            fixture.now
        ],
    )?;
    let other_candidate_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO memory_operation_log(operation, planner_version, actor, source,
                                         source_candidate_id, result_memory_id,
                                         confidence, reason, created_at_epoch)
         VALUES ('add', 'graph-schema-test', 'test', 'memory_candidate',
                 ?1, ?2, 0.9, 'mismatched provenance', ?3)",
        params![other_candidate_id, fixture.memory_id, fixture.now],
    )?;
    let mismatched_operation_id = conn.last_insert_rowid();
    let err = conn
        .execute(
            "INSERT INTO graph_edges
             (edge_type, edge_trust, from_node_kind, from_node_id, to_node_kind, to_node_id,
              source_event_ids, source_candidate_id, source_operation_id, confidence, reason,
              created_at_epoch)
             VALUES ('duplicates', 'trusted', 'memory', ?1, 'memory', ?2,
                     ?3, ?4, ?5, 0.9, 'mismatched operation', ?6)",
            params![
                fixture.memory_id,
                fixture.other_memory_id,
                format!("[{}]", fixture.episode_id),
                fixture.candidate_id,
                mismatched_operation_id,
                fixture.now
            ],
        )
        .expect_err("operation provenance must reference the same source candidate");
    assert!(err
        .to_string()
        .contains("graph_edges source candidate missing"));

    let err = conn
        .execute(
            "UPDATE graph_edges SET source_operation_id = ?1 WHERE id = ?2",
            params![mismatched_operation_id, edge_id],
        )
        .expect_err("retagging edge operation to a different candidate must fail closed");
    assert!(err
        .to_string()
        .contains("graph_edges source candidate missing"));

    let err = conn
        .execute(
            "UPDATE graph_edges SET source_candidate_id = ?1 WHERE id = ?2",
            params![fixture.candidate_id + 10_000, edge_id],
        )
        .expect_err("raw SQL graph source_candidate_id update must fail closed");
    assert!(err
        .to_string()
        .contains("graph_edges source candidate missing"));

    let err = conn
        .execute(
            "UPDATE memory_candidates SET id = ?1 WHERE id = ?2",
            params![fixture.candidate_id + 10_000, fixture.candidate_id],
        )
        .expect_err("updating a candidate used by graph_edges must fail closed");
    assert!(err
        .to_string()
        .contains("graph_edges source candidate in use"));

    let err = conn
        .execute(
            "UPDATE memory_operation_log SET source = 'graph_candidate' WHERE id = ?1",
            [fixture.operation_id],
        )
        .expect_err("retagging operation provenance used by graph_edges must fail closed");
    assert!(err
        .to_string()
        .contains("graph_edges source operation in use"));

    let err = conn
        .execute(
            "UPDATE memory_operation_log SET source_candidate_id = ?1 WHERE id = ?2",
            params![other_candidate_id, fixture.operation_id],
        )
        .expect_err("retagging operation candidate used by graph_edges must fail closed");
    assert!(err
        .to_string()
        .contains("graph_edges source operation in use"));

    let err = conn
        .execute(
            "DELETE FROM memory_candidates WHERE id = ?1",
            [fixture.candidate_id],
        )
        .expect_err("deleting a candidate used by graph_edges must fail closed");
    assert!(err
        .to_string()
        .contains("graph_edges source candidate in use"));

    conn.execute(
        "INSERT INTO graph_candidates(project_id, source_project, candidate_type, edge_type,
                                      from_ref, to_ref, evidence_event_ids, confidence,
                                      risk_class, reason, review_status, created_at_epoch,
                                      updated_at_epoch)
         VALUES (?1, '/tmp/remem-graph-schema', 'edge', 'duplicates',
                 'memory:1', 'memory:2', ?2, 0.9, 'low', 'graph candidate',
                 'approved', ?3, ?3)",
        params![
            fixture.project_id,
            format!("[{}]", fixture.episode_id),
            fixture.now
        ],
    )?;
    let graph_candidate_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO memory_operation_log(operation, planner_version, actor, source,
                                         source_candidate_id, result_memory_id,
                                         confidence, reason, created_at_epoch)
         VALUES ('add', 'graph-schema-test', 'test', 'graph_candidate',
                 ?1, ?2, 0.9, 'graph candidate provenance', ?3)",
        params![graph_candidate_id, fixture.memory_id, fixture.now],
    )?;
    let graph_operation_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO graph_edges
         (edge_type, edge_trust, from_node_kind, from_node_id, to_node_kind, to_node_id,
          source_event_ids, source_candidate_id, source_operation_id, confidence, reason,
          created_at_epoch)
         VALUES ('duplicates', 'trusted', 'memory', ?1, 'memory', ?2,
                 ?3, ?4, ?5, 0.9, 'valid graph candidate', ?6)",
        params![
            fixture.memory_id,
            fixture.other_memory_id,
            format!("[{}]", fixture.episode_id),
            graph_candidate_id,
            graph_operation_id,
            fixture.now
        ],
    )?;

    let err = conn
        .execute(
            "UPDATE graph_edges SET source_candidate_id = ?1 WHERE source_operation_id = ?2",
            params![graph_candidate_id + 10_000, graph_operation_id],
        )
        .expect_err("raw SQL graph candidate provenance update must fail closed");
    assert!(err
        .to_string()
        .contains("graph_edges source candidate missing"));

    let err = conn
        .execute(
            "UPDATE graph_candidates SET id = ?1 WHERE id = ?2",
            params![graph_candidate_id + 10_000, graph_candidate_id],
        )
        .expect_err("updating a graph candidate used by graph_edges must fail closed");
    assert!(err
        .to_string()
        .contains("graph_edges source candidate in use"));

    let err = conn
        .execute(
            "UPDATE memory_operation_log SET source = 'memory_candidate' WHERE id = ?1",
            [graph_operation_id],
        )
        .expect_err("retagging graph operation provenance used by graph_edges must fail closed");
    assert!(err
        .to_string()
        .contains("graph_edges source operation in use"));

    let err = conn
        .execute(
            "DELETE FROM graph_candidates WHERE id = ?1",
            [graph_candidate_id],
        )
        .expect_err("deleting a graph candidate used by graph_edges must fail closed");
    assert!(err
        .to_string()
        .contains("graph_edges source candidate in use"));

    Ok(())
}

#[test]
fn graph_edges_schema_installs_source_candidate_integrity_triggers() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    for trigger in [
        "graph_edges_validate_source_candidate_insert",
        "graph_edges_validate_source_candidate_update",
        "graph_edges_memory_candidates_delete",
        "graph_edges_memory_candidates_update_id",
        "graph_edges_graph_candidates_delete",
        "graph_edges_graph_candidates_update_id",
        "graph_edges_memory_operation_provenance_update",
    ] {
        let sql: String = conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
            [trigger],
            |row| row.get(0),
        )?;
        assert!(sql.contains("graph_edges"));
    }

    Ok(())
}

#[test]
fn memory_fact_invalidation_migration_upgrades_existing_fact_rows() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version <= 39)
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
        .filter(|migration| migration.version <= 39)
    {
        conn.execute(
            "INSERT INTO _schema_migrations (version, name, applied_at_epoch)
             VALUES (?1, ?2, ?3)",
            params![migration.version, migration.name, 1_700_000_000_i64],
        )?;
    }

    conn.execute(
        "INSERT INTO memory_facts
         (id, project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_event_ids, confidence, status, created_at_epoch,
          updated_at_epoch)
         VALUES (1, 'proj', 'deploy-target', 'affects_project', 'staging',
                 100, NULL, 110, '[]', 0.9, 'active', 120, 120)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_facts
         (id, project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_event_ids, confidence, status, created_at_epoch,
          updated_at_epoch)
         VALUES (2, 'proj', 'deploy-target', 'affects_project', 'staging',
                 50, 100, 60, '[]', 0.9, 'stale', 70, 220)",
        [],
    )?;

    run_migrations(&conn)?;

    let mut stmt = conn.prepare("PRAGMA table_info(memory_facts)")?;
    let mut rows = stmt.query([])?;
    let mut has_invalidated_at = false;
    while let Some(row) = rows.next()? {
        let column: String = row.get(1)?;
        if column == "invalidated_at_epoch" {
            has_invalidated_at = true;
            break;
        }
    }
    assert!(
        has_invalidated_at,
        "v040 must add memory_facts.invalidated_at_epoch"
    );

    let active_invalidated_at: Option<i64> = conn.query_row(
        "SELECT invalidated_at_epoch FROM memory_facts WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(active_invalidated_at, None);

    let stale_invalidated_at: Option<i64> = conn.query_row(
        "SELECT invalidated_at_epoch FROM memory_facts WHERE id = 2",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(stale_invalidated_at, Some(220));

    let index_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'index' AND name = 'idx_memory_facts_invalidated'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(index_count, 1);
    Ok(())
}

#[test]
fn run_migrations_rejects_db_newer_than_binary() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    run_migrations(&conn)?;

    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO _schema_migrations (version, name, applied_at_epoch) VALUES (?1, ?2, ?3)",
        params![99i64, "future_feature", now],
    )?;

    let err = run_migrations(&conn).expect_err("re-running on a newer DB must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("v99")
            && msg.contains(&format!("schema v{}", super::latest_schema_version()))
            && msg.contains("remem --version")
            && msg.contains("upgrade"),
        "error should mention the newer schema, binary schema, and verification command: {msg}"
    );
    Ok(())
}
