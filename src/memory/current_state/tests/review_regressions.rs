use anyhow::Result;
use rusqlite::params;

use super::super::{current_state, CurrentStateRequest};
use super::support::{
    current_state_test_conn, insert_current_state_memory_at, insert_state_key, set_current_memory,
};

#[test]
fn as_of_lookup_ignores_state_keys_created_after_cutoff() -> Result<()> {
    let conn = current_state_test_conn()?;
    conn.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, state_label,
          state_status, current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (10, 'repo', '/repo', 'decision', 'deploy-target',
                 'deploy target', 'active', NULL, 100, 100)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, state_label,
          state_status, current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (11, 'user', 'user:default', 'decision', 'deploy-target',
                 'future global deploy target', 'active', NULL, 300, 300)",
        [],
    )?;
    insert_current_state_memory_at(
        &conn,
        1,
        "/repo",
        "Repo deploy target",
        "Use staging.",
        "active",
        10,
        100,
        Some(100),
        None,
    )?;
    insert_current_state_memory_at(
        &conn,
        2,
        "/repo",
        "Future global deploy target",
        "Use production.",
        "active",
        11,
        300,
        Some(300),
        None,
    )?;
    conn.execute(
        "UPDATE memory_state_keys SET current_memory_id = ?1 WHERE id = ?2",
        params![1_i64, 10_i64],
    )?;
    conn.execute(
        "UPDATE memory_state_keys SET current_memory_id = ?1 WHERE id = ?2",
        params![2_i64, 11_i64],
    )?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(200),
            state_key: "deploy-target".to_string(),
            project: Some("/repo".to_string()),
            memory_type: Some("decision".to_string()),
            include_history: true,
            ..Default::default()
        },
    )?;

    assert_eq!(result.status, "current");
    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(1));
    assert!(result.matches.is_empty());
    Ok(())
}

#[test]
fn as_of_excludes_open_ended_stale_rows_after_stale_update() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_at(
        &conn,
        1,
        "/repo",
        "Manual stale deploy target",
        "Use staging.",
        "stale",
        10,
        100,
        Some(100),
        None,
    )?;
    conn.execute(
        "UPDATE memories SET updated_at_epoch = 200 WHERE id = 1",
        [],
    )?;
    set_current_memory(&conn, 1)?;

    let before_stale = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(150),
            state_key: "deploy-target".to_string(),
            project: Some("/repo".to_string()),
            memory_type: Some("decision".to_string()),
            include_history: true,
            ..Default::default()
        },
    )?;
    let after_stale = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(250),
            state_key: "deploy-target".to_string(),
            project: Some("/repo".to_string()),
            memory_type: Some("decision".to_string()),
            include_history: true,
            ..Default::default()
        },
    )?;

    assert_eq!(before_stale.status, "current");
    assert_eq!(
        before_stale.current.as_ref().map(|memory| memory.id),
        Some(1)
    );
    assert_eq!(after_stale.status, "no_current");
    assert!(after_stale.current.is_none());
    Ok(())
}

#[test]
fn explicit_owner_as_of_uses_distinct_owner_key_parameter() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_at(
        &conn,
        1,
        "/repo",
        "Repo deploy target",
        "Use staging.",
        "active",
        10,
        100,
        Some(100),
        None,
    )?;
    set_current_memory(&conn, 1)?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(150),
            state_key: "deploy-target".to_string(),
            owner_scope: Some("repo".to_string()),
            owner_key: Some("/repo".to_string()),
            memory_type: Some("decision".to_string()),
            include_history: true,
            ..Default::default()
        },
    )?;

    assert_eq!(result.status, "current");
    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(1));
    Ok(())
}

#[test]
fn as_of_lookup_does_not_return_active_row_updated_after_cutoff() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_at(
        &conn,
        1,
        "/repo",
        "Initial deploy target",
        "Use staging.",
        "active",
        10,
        100,
        Some(100),
        None,
    )?;
    conn.execute(
        "UPDATE memories
         SET title = 'Updated deploy target',
             content = 'Use production.',
             updated_at_epoch = 300
         WHERE id = 1",
        [],
    )?;
    set_current_memory(&conn, 1)?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(150),
            state_key: "deploy-target".to_string(),
            project: Some("/repo".to_string()),
            memory_type: Some("decision".to_string()),
            include_history: true,
            ..Default::default()
        },
    )?;

    assert_eq!(result.status, "no_current");
    assert!(result.current.is_none());
    Ok(())
}

#[test]
fn current_lookup_respects_memory_validity_window() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    let now = chrono::Utc::now().timestamp();
    insert_current_state_memory_at(
        &conn,
        1,
        "/repo",
        "Future deploy target",
        "Use production later.",
        "active",
        10,
        now,
        Some(now + 100),
        None,
    )?;
    insert_current_state_memory_at(
        &conn,
        2,
        "/repo",
        "Expired deploy target",
        "Use staging before cutoff.",
        "active",
        10,
        now - 200,
        Some(now - 200),
        Some(now - 100),
    )?;

    set_current_memory(&conn, 1)?;
    let future_result = current_state(
        &conn,
        &CurrentStateRequest {
            state_key: "deploy-target".to_string(),
            project: Some("/repo".to_string()),
            memory_type: Some("decision".to_string()),
            include_history: true,
            ..Default::default()
        },
    )?;

    set_current_memory(&conn, 2)?;
    let expired_result = current_state(
        &conn,
        &CurrentStateRequest {
            state_key: "deploy-target".to_string(),
            project: Some("/repo".to_string()),
            memory_type: Some("decision".to_string()),
            include_history: true,
            ..Default::default()
        },
    )?;

    assert_eq!(future_result.status, "no_current");
    assert!(future_result.current.is_none());
    assert_eq!(expired_result.status, "no_current");
    assert!(expired_result.current.is_none());
    Ok(())
}

#[test]
fn current_conflicts_ignore_future_active_rivals() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    let now = chrono::Utc::now().timestamp();
    insert_current_state_memory_at(
        &conn,
        1,
        "/repo",
        "Deploy target",
        "Use production.",
        "active",
        10,
        now - 100,
        Some(now - 100),
        None,
    )?;
    insert_current_state_memory_at(
        &conn,
        2,
        "/repo",
        "Future rival deploy target",
        "Use canary later.",
        "active",
        10,
        now,
        Some(now + 100),
        None,
    )?;
    set_current_memory(&conn, 1)?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, reason, created_at_epoch)
         VALUES ('conflicts', 2, 1, 10, 'future rival', ?1)",
        params![now],
    )?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            state_key: "deploy-target".to_string(),
            project: Some("/repo".to_string()),
            memory_type: Some("decision".to_string()),
            include_history: true,
            ..Default::default()
        },
    )?;

    assert_eq!(result.status, "current");
    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(1));
    assert!(result.conflicts.is_empty());
    Ok(())
}

#[test]
fn current_conflict_refs_include_edge_evidence() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_at(
        &conn,
        1,
        "/repo",
        "Deploy target",
        "Use production.",
        "active",
        10,
        100,
        Some(100),
        None,
    )?;
    insert_current_state_memory_at(
        &conn,
        2,
        "/repo",
        "Conflicting deploy target",
        "Use staging.",
        "active",
        10,
        110,
        Some(110),
        None,
    )?;
    set_current_memory(&conn, 1)?;
    conn.execute(
        "INSERT INTO memory_candidates
         (id, project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (30, NULL, 'project', 'decision', 'deploy-target',
                 'Use staging conflicts with production.', '[21,22]', 0.8,
                 'medium', 'approved', 115, 115)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_operation_log
         (id, operation, planner_version, actor, source, owner_scope, owner_key,
          memory_type, state_key, source_candidate_id, result_memory_id, created_at_epoch)
         VALUES (40, 'conflict', 'test', 'test', 'test', 'repo', '/repo',
                 'decision', 'deploy-target', 30, 1, 118)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, evidence_event_ids,
          source_candidate_id, source_operation_id, reason, created_at_epoch)
         VALUES ('conflicts', 2, 1, 10, '[21,22]', 30, 40,
                 'operator conflict', 120)",
        [],
    )?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            state_key: "deploy-target".to_string(),
            project: Some("/repo".to_string()),
            memory_type: Some("decision".to_string()),
            include_history: true,
            ..Default::default()
        },
    )?;

    assert_eq!(result.status, "unresolved_conflict");
    assert_eq!(result.conflicts.len(), 1);
    assert_eq!(result.conflicts[0].id, 2);
    assert_eq!(result.conflicts[0].relation.as_deref(), Some("conflicts"));
    assert_eq!(
        result.conflicts[0].reason.as_deref(),
        Some("operator conflict")
    );
    assert_eq!(result.conflicts[0].evidence_event_ids, vec![21, 22]);
    assert_eq!(result.conflicts[0].source_candidate_id, Some(30));
    assert_eq!(result.conflicts[0].source_operation_id, Some(40));
    Ok(())
}
