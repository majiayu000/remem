use anyhow::Result;
use rusqlite::{params, Connection};

use super::{
    activity_stats, get_turn, list_activity_sessions, list_turns, project_session,
    SessionActivityKey, PROJECTION_VERSION,
};

fn setup() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    crate::migrate::run_migrations(&conn)?;
    conn.execute(
        "INSERT INTO workspaces
         (id, root_path, created_at_epoch, updated_at_epoch)
         VALUES (1, '/repo', 1, 1)",
        [],
    )?;
    conn.execute(
        "INSERT INTO projects
         (id, workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES (1, 1, '/repo', 'demo/repo', 1, 1)",
        [],
    )?;
    conn.execute(
        "INSERT INTO sessions
         (id, host_id, workspace_id, project_id, session_id, started_at_epoch,
          last_seen_at_epoch, status)
         SELECT 1, id, 1, 1, 'session-1', 100, 180, 'completed'
         FROM hosts WHERE name = 'codex-cli'",
        [],
    )?;
    Ok(conn)
}

fn key() -> SessionActivityKey {
    SessionActivityKey {
        source_root: "local".to_string(),
        project: "demo/repo".to_string(),
        session_id: "session-1".to_string(),
    }
}

fn insert_message(conn: &Connection, id: i64, role: &str, content: &str, epoch: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO raw_messages
         (id, session_id, project, role, content, content_hash, source, cwd,
          created_at_epoch, source_root, event_time_source)
         VALUES (?1, 'session-1', 'demo/repo', ?2, ?3, ?4, 'transcript',
                 '/repo', ?5, 'local', 'transcript_event')",
        params![id, role, content, format!("hash-{id}"), epoch],
    )?;
    Ok(())
}

fn insert_action(
    conn: &Connection,
    id: i64,
    event_type: &str,
    tool: &str,
    epoch: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO captured_events
         (id, host_id, workspace_id, project_id, session_row_id, session_id,
          event_id, event_type, tool_name, content_hash, token_estimate,
          retention_class, created_at_epoch, inserted_at_epoch)
         SELECT ?1, h.id, 1, 1, 1, 'session-1', ?2, ?3, ?4, ?5, 0,
                'metadata', ?6, ?6
         FROM hosts h WHERE h.name = 'codex-cli'",
        params![
            id,
            format!("event-{id}"),
            event_type,
            tool,
            format!("event-hash-{id}"),
            epoch
        ],
    )?;
    Ok(())
}

#[test]
fn migration_creates_session_observatory_shape() -> Result<()> {
    let conn = setup()?;
    for object in ["session_turns", "session_turn_actions"] {
        let exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [object],
            |row| row.get(0),
        )?;
        assert_eq!(exists, 1, "missing table {object}");
    }
    assert_eq!(crate::migrate::latest_schema_version(), 84);
    Ok(())
}

#[test]
fn projects_ordered_turns_with_actions_and_is_idempotent() -> Result<()> {
    let mut conn = setup()?;
    insert_message(&conn, 10, "user", "Fix the login failure", 100)?;
    insert_message(
        &conn,
        11,
        "assistant",
        "I will reproduce the login failure and inspect the authentication path.",
        110,
    )?;
    insert_action(&conn, 20, "bash", "Bash", 120)?;
    insert_message(
        &conn,
        12,
        "assistant",
        "The login failure is fixed and tests pass.",
        130,
    )?;
    insert_message(&conn, 13, "user", "Explain the root cause", 140)?;
    insert_message(
        &conn,
        14,
        "assistant",
        "The empty password reached the database query.",
        150,
    )?;

    let first = project_session(&mut conn, &key(), 200)?;
    assert!(first.changed);
    assert_eq!(first.turn_count, 2);
    let second = project_session(&mut conn, &key(), 201)?;
    assert!(!second.changed);
    assert_eq!(second.source_digest, first.source_digest);

    let rows: Vec<(i64, String, String, String)> = conn
        .prepare(
            "SELECT turn_index, result_status, capture_health, understanding
             FROM session_turns ORDER BY turn_index",
        )?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, 1);
    assert_eq!(rows[0].1, "done");
    assert_eq!(rows[0].2, "partial");
    assert!(rows[0].3.contains("reproduce"));
    assert_eq!(rows[1].1, "answered");

    let action: (String, String, i64) = conn.query_row(
        "SELECT kind, summary, event_row_id FROM session_turn_actions",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(action.0, "run");
    assert!(action.1.contains("Bash"));
    assert_eq!(action.2, 20);

    let sessions = list_activity_sessions(&conn, Some("demo/repo"), None, 10)?;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].projected_turn_count, 2);
    let turns = list_turns(
        &conn,
        Some("demo/repo"),
        Some("local"),
        Some("session-1"),
        None,
        10,
    )?;
    assert_eq!(turns.len(), 2);
    let first_id = turns[1].turn.id.expect("stored turn id");
    assert_eq!(
        get_turn(&conn, first_id)?.expect("turn").turn.actions.len(),
        1
    );
    let stats = activity_stats(&conn, Some("demo/repo"), Some(90), Some(200))?;
    assert_eq!(stats.sessions, 1);
    assert_eq!(stats.turns, 2);
    assert_eq!(stats.actions, 1);
    assert_eq!(stats.projects[0].key, "demo/repo");
    assert_eq!(stats.tools[0].key, "Bash");
    Ok(())
}

#[test]
fn missing_understanding_and_session_link_remain_explicit() -> Result<()> {
    let mut conn = setup()?;
    conn.execute("DELETE FROM sessions", [])?;
    insert_message(&conn, 10, "user", "What does this module do?", 100)?;
    insert_message(&conn, 11, "assistant", "Okay.", 110)?;

    project_session(&mut conn, &key(), 200)?;
    let row: (Option<String>, String, String, i64) = conn.query_row(
        "SELECT understanding, capture_health, result_status, projection_version
         FROM session_turns",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(row.0, None);
    assert_eq!(row.1, "unavailable");
    assert_eq!(row.2, "answered");
    assert_eq!(row.3, PROJECTION_VERSION);
    Ok(())
}

#[test]
fn source_change_replaces_only_the_exact_session_projection() -> Result<()> {
    let mut conn = setup()?;
    insert_message(&conn, 10, "user", "Initial request", 100)?;
    insert_message(
        &conn,
        11,
        "assistant",
        "This response is long enough to be meaningful.",
        110,
    )?;
    project_session(&mut conn, &key(), 200)?;
    let original_digest: String =
        conn.query_row("SELECT source_digest FROM session_turns", [], |row| {
            row.get(0)
        })?;

    insert_message(&conn, 12, "user", "Follow-up request", 120)?;
    insert_message(
        &conn,
        13,
        "assistant",
        "This follow-up response is also meaningful.",
        130,
    )?;
    let updated = project_session(&mut conn, &key(), 300)?;
    assert!(updated.changed);
    assert_ne!(updated.source_digest, original_digest);
    assert_eq!(updated.turn_count, 2);
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM session_turns", [], |row| row.get(0))?;
    assert_eq!(count, 2);
    Ok(())
}
