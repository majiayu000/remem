use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use super::{
    activity_stats, get_turn, list_activity_sessions, list_turns, project_session,
    SessionActivityKey, PROJECTION_VERSION,
};

fn setup() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    crate::migrate::run_migrations(&conn)?;
    conn.execute_batch(
        "INSERT INTO raw_session_identities (
            id, source_root, transcript_path, fallback_session_id,
            canonical_session_id, project, legacy_project, status,
            observed_mtime_ns, observed_size_bytes, first_seen_at_epoch,
            last_seen_at_epoch
         ) VALUES (1, 'local', '/home/test/.codex/sessions/session-1.jsonl',
                   'session-1', 'session-1', '/repo', 'repo', 'active',
                   1, 1, 1, 1);",
    )?;
    conn.execute(
        "INSERT INTO workspaces
         (id, root_path, created_at_epoch, updated_at_epoch)
         VALUES (1, '/repo', 1, 1)",
        [],
    )?;
    conn.execute(
        "INSERT INTO projects
         (id, workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES (1, 1, '/repo', 'repo', 1, 1)",
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
        project: "/repo".to_string(),
        session_id: "session-1".to_string(),
    }
}

fn insert_message(conn: &Connection, id: i64, role: &str, content: &str, epoch: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO raw_messages
         (id, session_id, project, role, content, content_hash, source, cwd,
          created_at_epoch, source_root, event_time_source,
          transcript_identity_id, transcript_record_ordinal)
         VALUES (?1, 'session-1', '/repo', ?2, ?3, ?4, 'transcript',
                 '/repo', ?5, 'local', 'transcript_event', 1, ?1)",
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
          retention_class, created_at_epoch, inserted_at_epoch, reference_time_epoch)
         SELECT ?1, h.id, 1, 1, 1, 'session-1', ?2, ?3, ?4, ?5, 0,
                'metadata', ?6, ?6, ?6
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
    for index in [
        "idx_session_turns_recent",
        "idx_session_turns_project_recent",
        "idx_session_turns_session",
        "idx_session_turns_identity",
        "idx_session_turn_actions_turn",
        "idx_session_turn_actions_event",
        "idx_raw_messages_activity_tuple_recent",
        "idx_raw_messages_activity_recent",
        "idx_raw_messages_project_activity_recent",
    ] {
        let exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
            [index],
            |row| row.get(0),
        )?;
        assert_eq!(exists, 1, "missing index {index}");
    }
    assert!(conn
        .execute(
            "INSERT INTO session_turn_actions
             (session_turn_id, action_index, kind, summary, files_json, created_at_epoch)
             VALUES (999, 0, 'invalid', 'bad', '[]', 1)",
            [],
        )
        .is_err());
    assert_eq!(crate::migrate::latest_schema_version(), 85);
    for (sql, expected_index) in [
        (
            "EXPLAIN QUERY PLAN SELECT id FROM raw_messages
             ORDER BY created_at_epoch DESC, id DESC LIMIT 50",
            "idx_raw_messages_activity_recent",
        ),
        (
            "EXPLAIN QUERY PLAN SELECT id FROM raw_messages
             WHERE project = '/repo'
             ORDER BY created_at_epoch DESC, id DESC LIMIT 50",
            "idx_raw_messages_project_activity_recent",
        ),
    ] {
        let plan = conn
            .prepare(sql)?
            .query_map([], |row| row.get::<_, String>(3))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .join(" ");
        assert!(
            plan.contains(expected_index),
            "activity candidate scan must use {expected_index}: {plan}"
        );
    }
    Ok(())
}

#[test]
fn action_constraints_cascade_and_bounded_reads_are_enforced() -> Result<()> {
    let mut conn = setup()?;
    insert_message(&conn, 10, "user", "Exercise action bounds", 100)?;
    insert_message(
        &conn,
        11,
        "assistant",
        "This response creates a valid projected turn for constraint tests.",
        110,
    )?;
    project_session(&mut conn, &key(), 200)?;
    let turn_id: i64 = conn.query_row("SELECT id FROM session_turns", [], |row| row.get(0))?;
    assert!(conn
        .execute(
            "INSERT INTO session_turn_actions
             (session_turn_id, action_index, kind, summary, files_json, created_at_epoch)
             VALUES (?1, 1, 'invalid', 'bad', '[]', 1)",
            [turn_id],
        )
        .is_err());
    for index in 1..=101_i64 {
        conn.execute(
            "INSERT INTO session_turn_actions
             (session_turn_id, action_index, kind, summary, files_json, created_at_epoch)
             VALUES (?1, ?2, 'read', 'bounded action', '[]', ?2)",
            params![turn_id, index],
        )?;
    }
    let detail = get_turn(&conn, turn_id)?.context("turn detail")?;
    assert_eq!(detail.turn.actions.len(), 100);
    assert!(detail.turn.actions_truncated);
    conn.execute("DELETE FROM session_turns WHERE id = ?1", [turn_id])?;
    let remaining: i64 = conn.query_row(
        "SELECT COUNT(*) FROM session_turn_actions WHERE session_turn_id = ?1",
        [turn_id],
        |row| row.get(0),
    )?;
    assert_eq!(remaining, 0);
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

    let sessions = list_activity_sessions(&conn, Some("/repo"), None, 10)?;
    assert_eq!(sessions.data.len(), 1);
    assert_eq!(sessions.data[0].projected_turn_count, 2);
    let turns = list_turns(
        &conn,
        Some("/repo"),
        Some("local"),
        Some("session-1"),
        None,
        10,
    )?;
    assert_eq!(turns.data.len(), 2);
    let first_id = turns.data[1].turn.id.expect("stored turn id");
    assert_eq!(
        get_turn(&conn, first_id)?.expect("turn").turn.actions.len(),
        1
    );
    let stats = activity_stats(&conn, Some("/repo"), Some(90), Some(200))?;
    assert_eq!(stats.sessions, 1);
    assert_eq!(stats.turns, 2);
    assert_eq!(stats.actions, 1);
    assert_eq!(stats.projects[0].key, "/repo");
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

#[test]
fn projection_uses_exact_project_identity_and_redacts_visible_text() -> Result<()> {
    let mut conn = setup()?;
    conn.execute(
        "INSERT INTO workspaces
         (id, root_path, created_at_epoch, updated_at_epoch)
         VALUES (2, '/other/repo', 1, 1)",
        [],
    )?;
    conn.execute(
        "INSERT INTO projects
         (id, workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES (2, 2, '/other/repo', 'repo', 1, 1)",
        [],
    )?;
    conn.execute(
        "INSERT INTO sessions
         (id, host_id, workspace_id, project_id, session_id, started_at_epoch,
          last_seen_at_epoch, status)
         SELECT 2, id, 2, 2, 'session-1', 100, 180, 'completed'
         FROM hosts WHERE name = 'codex-cli'",
        [],
    )?;
    insert_message(
        &conn,
        10,
        "user",
        "OPENAI_API_KEY=sk-user-secret\nUse Bearer assistant-secret",
        100,
    )?;
    insert_message(
        &conn,
        11,
        "assistant",
        "Authorization: Bearer assistant-secret",
        110,
    )?;
    insert_action(&conn, 20, "bash", "Bash", 120)?;
    insert_message(
        &conn,
        12,
        "assistant",
        "Completed with Authorization: Bearer result-secret",
        130,
    )?;

    project_session(&mut conn, &key(), 200)?;
    let item = list_turns(
        &conn,
        Some("/repo"),
        Some("local"),
        Some("session-1"),
        None,
        10,
    )?
    .data
    .pop()
    .context("projected turn")?;
    let serialized = serde_json::to_string(&item)?;
    assert!(!serialized.contains("sk-user-secret"));
    assert!(!serialized.contains("assistant-secret"));
    assert!(!serialized.contains("result-secret"));
    assert!(serialized.contains("REDACTED"));
    assert_eq!(item.turn.actions.len(), 1);
    Ok(())
}

#[test]
fn action_without_post_action_result_is_aborted_and_equal_boundaries_are_unassigned() -> Result<()>
{
    let mut conn = setup()?;
    insert_message(&conn, 10, "user", "Investigate the failure", 100)?;
    insert_message(
        &conn,
        11,
        "assistant",
        "I will inspect the failing command before reporting a result.",
        110,
    )?;
    insert_action(&conn, 20, "bash", "Bash", 119)?;
    insert_action(&conn, 21, "bash", "Bash", 120)?;
    insert_message(&conn, 12, "user", "Continue", 120)?;
    insert_message(
        &conn,
        13,
        "assistant",
        "I can continue after the ambiguous timestamp boundary.",
        130,
    )?;

    project_session(&mut conn, &key(), 200)?;
    let rows: Vec<(i64, String, Option<String>, String)> = conn
        .prepare(
            "SELECT turn_index, result_status, result_summary, capture_health
             FROM session_turns ORDER BY turn_index",
        )?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(
        rows[0],
        (1, "aborted".to_string(), None, "unavailable".to_string())
    );
    let action_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM session_turn_actions", [], |row| {
            row.get(0)
        })?;
    assert_eq!(action_count, 1);
    Ok(())
}

#[test]
fn source_digest_changes_when_session_linkage_changes() -> Result<()> {
    let mut conn = setup()?;
    conn.execute("DELETE FROM sessions", [])?;
    insert_message(&conn, 10, "user", "Link this session", 100)?;
    insert_message(
        &conn,
        11,
        "assistant",
        "This response is long enough to produce a stable projection.",
        110,
    )?;
    let first = project_session(&mut conn, &key(), 200)?;
    conn.execute(
        "INSERT INTO sessions
         (id, host_id, workspace_id, project_id, session_id, started_at_epoch,
          last_seen_at_epoch, status)
         SELECT 1, id, 1, 1, 'session-1', 100, 180, 'completed'
         FROM hosts WHERE name = 'codex-cli'",
        [],
    )?;
    let second = project_session(&mut conn, &key(), 201)?;
    assert!(second.changed);
    assert_ne!(first.source_digest, second.source_digest);
    let linked: Option<i64> =
        conn.query_row("SELECT session_row_id FROM session_turns", [], |row| {
            row.get(0)
        })?;
    assert_eq!(linked, Some(1));
    Ok(())
}

#[test]
fn activity_session_cursor_preserves_equal_epoch_tuples() -> Result<()> {
    let conn = setup()?;
    for (id, project, session) in [(10, "/a", "s-a"), (11, "/b", "s-b"), (12, "/c", "s-c")] {
        conn.execute(
            "INSERT INTO raw_messages
             (id, session_id, project, role, content, content_hash, source, cwd,
              created_at_epoch, source_root, event_time_source)
             VALUES (?1, ?2, ?3, 'user', 'request', ?4, 'transcript', ?3,
                     100, 'local', 'transcript_event')",
            params![id, session, project, format!("hash-{id}")],
        )?;
    }
    let first = list_activity_sessions(&conn, None, None, 2)?;
    assert!(first.has_more);
    assert_eq!(first.data.len(), 2);
    let second = list_activity_sessions(&conn, None, first.next_cursor.as_deref(), 2)?;
    assert!(!second.has_more);
    assert_eq!(second.data.len(), 1);
    assert_eq!(second.data[0].project, "/a");
    Ok(())
}

#[test]
fn activity_session_scan_is_bounded_and_sparse_pages_advance() -> Result<()> {
    let conn = setup()?;
    for id in 10..=80_i64 {
        conn.execute(
            "INSERT INTO raw_messages
             (id, session_id, project, role, content, content_hash, source, cwd,
              created_at_epoch, source_root, event_time_source)
             VALUES (?1, 'large', '/large', 'assistant', 'message', ?2,
                     'transcript', '/large', ?1, 'local', 'transcript_event')",
            params![id, format!("hash-{id}")],
        )?;
    }
    conn.execute(
        "INSERT INTO raw_messages
         (id, session_id, project, role, content, content_hash, source, cwd,
          created_at_epoch, source_root, event_time_source)
         VALUES (9, 'older', '/older', 'user', 'request', 'older-hash',
                 'transcript', '/older', 9, 'local', 'transcript_event')",
        [],
    )?;

    let first = list_activity_sessions(&conn, None, None, 1)?;
    assert_eq!(first.data[0].session_id, "large");
    assert!(first.has_more);
    let sparse = list_activity_sessions(&conn, None, first.next_cursor.as_deref(), 1)?;
    assert!(sparse.data.is_empty());
    assert!(sparse.has_more);
    assert_ne!(sparse.next_cursor, first.next_cursor);
    let final_page = list_activity_sessions(&conn, None, sparse.next_cursor.as_deref(), 1)?;
    assert_eq!(final_page.data[0].session_id, "older");
    assert!(!final_page.has_more);
    Ok(())
}

#[test]
fn activity_session_message_counts_are_bounded_and_explicitly_truncated() -> Result<()> {
    let conn = setup()?;
    conn.execute_batch("BEGIN")?;
    for id in 10..=10_010_i64 {
        conn.execute(
            "INSERT INTO raw_messages
             (id, session_id, project, role, content, content_hash, source, cwd,
              created_at_epoch, source_root, event_time_source)
             VALUES (?1, 'huge', '/huge', 'assistant', 'message', ?2,
                     'transcript', '/huge', ?1, 'local', 'transcript_event')",
            params![id, format!("huge-hash-{id}")],
        )?;
    }
    conn.execute_batch("COMMIT")?;

    let page = list_activity_sessions(&conn, Some("/huge"), None, 1)?;
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].message_count, 10_000);
    assert_eq!(page.data[0].assistant_message_count, 10_000);
    assert!(page.data[0].message_counts_truncated);
    assert_eq!(page.data[0].first_epoch, None);
    Ok(())
}

#[test]
fn non_local_source_root_never_links_local_captured_actions() -> Result<()> {
    let mut conn = setup()?;
    conn.execute(
        "INSERT INTO raw_messages
         (id, session_id, project, role, content, content_hash, source, cwd,
          created_at_epoch, source_root, event_time_source)
         VALUES
         (10, 'session-1', '/repo', 'user', 'Remote request', 'remote-user',
          'transcript', '/repo', 100, 'remote-root', 'transcript_event'),
         (11, 'session-1', '/repo', 'assistant', 'Remote answer is complete.',
          'remote-answer', 'transcript', '/repo', 130, 'remote-root',
          'transcript_event')",
        [],
    )?;
    insert_action(&conn, 20, "bash", "Bash", 120)?;
    let remote_key = SessionActivityKey {
        source_root: "remote-root".to_string(),
        ..key()
    };
    project_session(&mut conn, &remote_key, 200)?;
    let (health, actions): (String, i64) = conn.query_row(
        "SELECT capture_health,
                (SELECT COUNT(*) FROM session_turn_actions)
         FROM session_turns",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(health, "unavailable");
    assert_eq!(actions, 0);
    Ok(())
}

#[test]
fn local_transcript_never_borrows_actions_from_a_different_host() -> Result<()> {
    let mut conn = setup()?;
    conn.execute(
        "UPDATE raw_session_identities
         SET transcript_path = '/home/test/.claude/projects/session-1.jsonl'
         WHERE id = 1",
        [],
    )?;
    insert_message(&conn, 10, "user", "Inspect this Claude session", 100)?;
    insert_action(&conn, 20, "bash", "Bash", 120)?;
    insert_message(
        &conn,
        11,
        "assistant",
        "The Claude-side inspection completed successfully.",
        130,
    )?;

    project_session(&mut conn, &key(), 200)?;
    let (session_row_id, health, action_count): (Option<i64>, String, i64) = conn.query_row(
        "SELECT session_row_id, capture_health,
                (SELECT COUNT(*) FROM session_turn_actions)
         FROM session_turns",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(session_row_id, None);
    assert_eq!(health, "unavailable");
    assert_eq!(action_count, 0);
    Ok(())
}

#[test]
fn late_reference_time_enrichment_reassigns_action_and_changes_digest() -> Result<()> {
    let mut conn = setup()?;
    insert_message(&conn, 10, "user", "First turn", 100)?;
    insert_message(&conn, 11, "assistant", "First response is complete.", 130)?;
    insert_message(&conn, 12, "user", "Second turn", 150)?;
    insert_message(&conn, 13, "assistant", "Second response is complete.", 180)?;
    insert_action(&conn, 20, "bash", "Bash", 170)?;
    conn.execute(
        "UPDATE captured_events SET reference_time_epoch = NULL WHERE id = 20",
        [],
    )?;
    let first = project_session(&mut conn, &key(), 200)?;
    let first_turn: i64 = conn.query_row(
        "SELECT t.turn_index FROM session_turn_actions a
         JOIN session_turns t ON t.id = a.session_turn_id",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(first_turn, 2);

    conn.execute(
        "UPDATE captured_events SET reference_time_epoch = 120 WHERE id = 20",
        [],
    )?;
    let second = project_session(&mut conn, &key(), 201)?;
    let second_turn: i64 = conn.query_row(
        "SELECT t.turn_index FROM session_turn_actions a
         JOIN session_turns t ON t.id = a.session_turn_id",
        [],
        |row| row.get(0),
    )?;
    assert!(second.changed);
    assert_ne!(first.source_digest, second.source_digest);
    assert_eq!(second_turn, 1);
    Ok(())
}

#[test]
fn action_first_turn_does_not_invent_understanding_and_reprojection_preserves_ids() -> Result<()> {
    let mut conn = setup()?;
    insert_message(&conn, 10, "user", "Run the verification", 100)?;
    insert_action(&conn, 20, "bash", "Bash", 110)?;
    insert_message(
        &conn,
        11,
        "assistant",
        "Verification completed successfully after the command.",
        120,
    )?;
    project_session(&mut conn, &key(), 200)?;
    let (first_id, understanding, result_id): (i64, Option<String>, Option<i64>) = conn.query_row(
        "SELECT id, understanding, result_message_id FROM session_turns",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(understanding, None);
    assert_eq!(result_id, Some(11));

    insert_message(&conn, 12, "user", "Explain the result", 130)?;
    insert_message(
        &conn,
        13,
        "assistant",
        "The verification command completed without an error.",
        140,
    )?;
    project_session(&mut conn, &key(), 201)?;
    let preserved_id: i64 = conn.query_row(
        "SELECT id FROM session_turns WHERE turn_index = 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(preserved_id, first_id);
    Ok(())
}
