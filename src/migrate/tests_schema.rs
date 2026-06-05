use anyhow::Result;
use rusqlite::{params, Connection};

use super::{run_migrations, MIGRATIONS};

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

    let err = conn
        .execute(
            "INSERT INTO graph_edges
             (edge_type, edge_trust, from_node_kind, from_node_id, to_node_kind, to_node_id,
              source_event_ids, source_candidate_id, source_operation_id, confidence, reason,
              created_at_epoch)
             VALUES ('duplicates', 'trusted', 'memory', ?1, 'memory', ?1,
                     ?2, ?3, ?4, 0.9, 'self edge', ?5)",
            params![
                memory_id,
                format!("[{episode_id}]"),
                candidate_id,
                operation_id,
                now
            ],
        )
        .expect_err("raw SQL graph self-edge must fail");
    assert!(err.to_string().contains("CHECK constraint failed"));

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
