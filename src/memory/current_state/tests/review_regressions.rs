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
