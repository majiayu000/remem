use anyhow::Result;
use rusqlite::{params, Connection};

use super::{current_state, CurrentStateRequest};

fn current_state_test_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

#[allow(clippy::too_many_arguments)]
fn insert_current_state_memory(
    conn: &Connection,
    id: i64,
    title: &str,
    content: &str,
    status: &str,
    state_key_id: i64,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
) -> Result<()> {
    insert_current_state_memory_at(
        conn,
        id,
        "/repo",
        title,
        content,
        status,
        state_key_id,
        1_700_000_000_i64 + id,
        valid_from_epoch,
        valid_to_epoch,
    )
}

#[allow(clippy::too_many_arguments)]
fn insert_current_state_memory_at(
    conn: &Connection,
    id: i64,
    project: &str,
    title: &str,
    content: &str,
    status: &str,
    state_key_id: i64,
    created_at_epoch: i64,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope, source_project,
          target_project, owner_scope, owner_key, context_class, valid_from_epoch,
          valid_to_epoch, state_key_id)
         VALUES (?1, NULL, ?2, 'deploy-target', ?3, ?4, 'decision', NULL,
                 ?5, ?5, ?6, NULL, 'project', ?2, ?2, 'repo', ?2,
                 'startup_core', ?7, ?8, ?9)",
        params![
            id,
            project,
            title,
            content,
            created_at_epoch,
            status,
            valid_from_epoch,
            valid_to_epoch,
            state_key_id
        ],
    )?;
    Ok(())
}

fn insert_state_key_for(
    conn: &Connection,
    id: i64,
    owner_key: &str,
    current_memory_id: Option<i64>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, state_label,
          state_status, current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (?1, 'repo', ?2, 'decision', 'deploy-target',
                 'deploy target', 'active', ?3, 1700000000, 1700000010)",
        params![id, owner_key, current_memory_id],
    )?;
    Ok(())
}

fn insert_state_key(conn: &Connection) -> Result<()> {
    insert_state_key_for(conn, 10, "/repo", None)
}

fn set_current_memory(conn: &Connection, current_memory_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE memory_state_keys SET current_memory_id = ?1 WHERE id = 10",
        [current_memory_id],
    )?;
    Ok(())
}

fn request() -> CurrentStateRequest {
    CurrentStateRequest {
        state_key: "deploy-target".to_string(),
        project: Some("/repo".to_string()),
        memory_type: Some("decision".to_string()),
        include_history: true,
        ..Default::default()
    }
}

#[test]
fn returns_current_answer_when_state_key_has_single_active_current() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        None,
        None,
    )?;
    set_current_memory(&conn, 2)?;

    let result = current_state(&conn, &request())?;

    assert_eq!(result.status, "current");
    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(2));
    assert!(result.conflicts.is_empty());
    Ok(())
}

#[test]
fn shows_superseded_history_and_edge_evidence_for_current_answer() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory(
        &conn,
        1,
        "Old deploy target",
        "Use staging.",
        "stale",
        10,
        Some(100),
        Some(200),
    )?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        Some(200),
        None,
    )?;
    set_current_memory(&conn, 2)?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, evidence_event_ids,
          reason, created_at_epoch)
         VALUES ('supersedes', 1, 2, 10, '[7,8]', 'new deploy decision', 300)",
        [],
    )?;

    let result = current_state(&conn, &request())?;

    assert_eq!(result.status, "current");
    assert_eq!(result.history.len(), 1);
    assert_eq!(result.history[0].id, 1);
    assert_eq!(result.history[0].relation.as_deref(), Some("supersedes"));
    assert_eq!(result.history[0].evidence_event_ids, vec![7, 8]);
    assert_eq!(result.why[0].reason.as_deref(), Some("new deploy decision"));
    Ok(())
}

#[test]
fn unresolved_active_conflict_does_not_silently_choose_current_pointer() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        None,
        None,
    )?;
    insert_current_state_memory(
        &conn,
        3,
        "Deploy target conflict",
        "Use staging.",
        "active",
        10,
        None,
        None,
    )?;
    set_current_memory(&conn, 2)?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, reason, created_at_epoch)
         VALUES ('conflicts', 3, 2, 10, 'operator conflict', 300)",
        [],
    )?;

    let result = current_state(&conn, &request())?;

    assert_eq!(result.status, "unresolved_conflict");
    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(2));
    assert_eq!(result.conflicts.len(), 1);
    assert_eq!(result.conflicts[0].id, 3);
    Ok(())
}

#[test]
fn as_of_time_uses_memory_validity_window() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory(
        &conn,
        1,
        "Old deploy target",
        "Use staging.",
        "stale",
        10,
        Some(100),
        Some(200),
    )?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        Some(200),
        None,
    )?;
    set_current_memory(&conn, 2)?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(150),
            ..request()
        },
    )?;

    assert_eq!(result.status, "current");
    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(1));
    assert_eq!(
        result.current.as_ref().map(|memory| memory.status.as_str()),
        Some("stale")
    );
    Ok(())
}

#[test]
fn omitted_project_defaults_to_current_repo_scope() -> Result<()> {
    let conn = current_state_test_conn()?;
    let cwd = std::env::current_dir()?;
    let current_project = crate::db::project_from_cwd(cwd.to_string_lossy().as_ref());
    insert_state_key_for(&conn, 10, &current_project, None)?;
    insert_state_key_for(&conn, 11, "/other/repo", None)?;
    insert_current_state_memory_at(
        &conn,
        2,
        &current_project,
        "Current repo deploy target",
        "Use production.",
        "active",
        10,
        100,
        None,
        None,
    )?;
    insert_current_state_memory_at(
        &conn,
        3,
        "/other/repo",
        "Other repo deploy target",
        "Use staging.",
        "active",
        11,
        100,
        None,
        None,
    )?;
    conn.execute(
        "UPDATE memory_state_keys SET current_memory_id = 2 WHERE id = 10",
        [],
    )?;
    conn.execute(
        "UPDATE memory_state_keys SET current_memory_id = 3 WHERE id = 11",
        [],
    )?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            project: None,
            ..request()
        },
    )?;

    assert_eq!(result.status, "current");
    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(2));
    assert!(result.matches.is_empty());
    Ok(())
}

#[test]
fn as_of_time_excludes_memories_created_after_cutoff_when_valid_from_is_null() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_at(
        &conn,
        1,
        "/repo",
        "Old deploy target",
        "Use staging.",
        "stale",
        10,
        100,
        None,
        Some(200),
    )?;
    insert_current_state_memory_at(
        &conn,
        2,
        "/repo",
        "Future deploy target",
        "Use production.",
        "active",
        10,
        300,
        None,
        None,
    )?;
    set_current_memory(&conn, 2)?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(150),
            ..request()
        },
    )?;

    assert_eq!(result.status, "current");
    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(1));
    Ok(())
}

#[test]
fn as_of_time_includes_archived_memories_inside_validity_window() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_at(
        &conn,
        1,
        "/repo",
        "Archived deploy target",
        "Use staging.",
        "archived",
        10,
        100,
        Some(100),
        Some(200),
    )?;
    insert_current_state_memory_at(
        &conn,
        2,
        "/repo",
        "Deploy target",
        "Use production.",
        "active",
        10,
        200,
        Some(200),
        None,
    )?;
    set_current_memory(&conn, 2)?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(150),
            ..request()
        },
    )?;

    assert_eq!(result.status, "current");
    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(1));
    assert_eq!(
        result.current.as_ref().map(|memory| memory.status.as_str()),
        Some("archived")
    );
    Ok(())
}

#[test]
fn current_answer_facts_exclude_expired_and_future_active_facts() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        None,
        None,
    )?;
    set_current_memory(&conn, 2)?;
    let now = chrono::Utc::now().timestamp();
    for (id, object, valid_from, valid_to) in [
        (1_i64, "production", now - 100, Some(now + 100)),
        (2_i64, "expired", now - 200, Some(now - 100)),
        (3_i64, "future", now + 100, None),
    ] {
        conn.execute(
            "INSERT INTO memory_facts
             (id, project, subject, predicate, object, valid_from_epoch,
              valid_to_epoch, learned_at_epoch, source_memory_id, source_event_ids,
              confidence, status, created_at_epoch, updated_at_epoch)
             VALUES (?1, '/repo', 'deploy-target', 'affects_project', ?2, ?3,
                     ?4, ?3, 2, '[]', 0.9, 'active', ?3, ?3)",
            params![id, object, valid_from, valid_to],
        )?;
    }

    let result = current_state(&conn, &request())?;

    assert_eq!(
        result
            .facts
            .iter()
            .map(|fact| fact.object.as_str())
            .collect::<Vec<_>>(),
        vec!["production"]
    );
    Ok(())
}
