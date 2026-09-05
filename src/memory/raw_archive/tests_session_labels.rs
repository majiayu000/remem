use rusqlite::{params, Connection};

use super::sessions::{list_sessions, RawSessionQuery};
use super::{insert_raw_message, ROLE_USER, SOURCE_HOOK};

const SHANGHAI_NEW_YEAR: i64 = 1_735_660_800;

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::migrate::run_migrations(&conn).unwrap();
    conn
}

fn insert_at_epoch(conn: &Connection, session_id: &str, project: &str, content: &str, epoch: i64) {
    let outcome = insert_raw_message(
        conn,
        session_id,
        project,
        ROLE_USER,
        content,
        SOURCE_HOOK,
        None,
        None,
    )
    .unwrap()
    .expect("insert must produce a row");
    conn.execute(
        "UPDATE raw_messages SET created_at_epoch = ?1 WHERE id = ?2",
        params![epoch, outcome.id],
    )
    .unwrap();
}

fn identify_raw_sessions(conn: &Connection, host: &str) {
    let tuples = {
        let mut statement = conn
            .prepare("SELECT DISTINCT source_root, project, session_id FROM raw_messages")
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    for (root, project, session) in tuples {
        conn.execute(
            "INSERT INTO raw_session_identities
             (source_root, transcript_path, host, fallback_session_id,
              canonical_session_id, project, legacy_project, status,
              contract_version, observed_mtime_ns, observed_size_bytes,
              first_seen_at_epoch, last_seen_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?5, 'active', 1, 1, 1, 1, 1)",
            params![
                root,
                format!("/tmp/.codex/sessions/{session}.jsonl"),
                host,
                session,
                project
            ],
        )
        .unwrap();
        let identity_id = conn.last_insert_rowid();
        conn.execute(
            "UPDATE raw_messages
             SET transcript_identity_id = ?1, transcript_record_ordinal = id
             WHERE source_root = ?2 AND project = ?3 AND session_id = ?4",
            params![identity_id, root, project, session],
        )
        .unwrap();
    }
}

fn list_project(conn: &Connection, project: &str) -> Vec<super::sessions::RawSessionSummary> {
    list_sessions(
        conn,
        &RawSessionQuery {
            since_epoch: None,
            until_epoch: None,
            project: Some(project.to_string()),
            sample_user_messages: 1,
            latest: None,
        },
    )
    .unwrap()
}

fn seed_host_project_session(
    conn: &Connection,
    host: &str,
    project: &str,
    session_id: &str,
    epoch: i64,
) -> i64 {
    let host_id: i64 = conn
        .query_row("SELECT id FROM hosts WHERE name = ?1", [host], |row| {
            row.get(0)
        })
        .unwrap();
    conn.execute(
        "INSERT INTO workspaces(root_path, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?2)",
        params![project, epoch],
    )
    .unwrap();
    let workspace_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO projects(workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![workspace_id, project, project.trim_start_matches('/'), epoch],
    )
    .unwrap();
    let project_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO sessions(host_id, workspace_id, project_id, session_id, started_at_epoch,
                              last_seen_at_epoch, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, 'active')",
        params![host_id, workspace_id, project_id, session_id, epoch],
    )
    .unwrap();
    conn.last_insert_rowid()
}

#[test]
fn list_sessions_without_summary_exposes_mmdd_and_abstains_from_intent() {
    let conn = setup_conn();
    insert_at_epoch(
        &conn,
        "s-bare",
        "/proj",
        "first question",
        SHANGHAI_NEW_YEAR,
    );
    identify_raw_sessions(&conn, "codex-cli");

    let sessions = list_project(&conn, "/proj");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].mmdd.as_deref(), Some("0101"));
    assert_eq!(sessions[0].session_intent, None);
    assert_eq!(sessions[0].session_topic, None);
    assert_eq!(sessions[0].display_label, None);
    assert_eq!(sessions[0].session_intent_source, None);
}

#[test]
fn list_sessions_joins_summary_intent_by_session_row_id() {
    let conn = setup_conn();
    insert_at_epoch(
        &conn,
        "s-linked",
        "/proj",
        "repair listing",
        SHANGHAI_NEW_YEAR,
    );
    identify_raw_sessions(&conn, "codex-cli");
    let session_row_id =
        seed_host_project_session(&conn, "codex-cli", "/proj", "s-linked", SHANGHAI_NEW_YEAR);
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, session_row_id, request, created_at_epoch,
          session_intent, session_topic, session_intent_source)
         VALUES ('s-linked', '/proj', ?1, 'ignored fallback', ?2, 'fix',
                 'Batch text display', 'summary')",
        params![session_row_id, SHANGHAI_NEW_YEAR],
    )
    .unwrap();

    let sessions = list_project(&conn, "/proj");
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].display_label.as_deref(),
        Some("0101｜fix｜Batch text display")
    );
    assert_eq!(sessions[0].session_intent.as_deref(), Some("fix"));
    assert_eq!(
        sessions[0].session_intent_source.as_deref(),
        Some("summary")
    );
}

#[test]
fn list_sessions_falls_back_to_memory_session_id_when_row_id_is_missing() {
    let conn = setup_conn();
    insert_at_epoch(
        &conn,
        "s-memory",
        "/proj",
        "fallback title",
        SHANGHAI_NEW_YEAR,
    );
    identify_raw_sessions(&conn, "codex-cli");
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, created_at_epoch,
          session_intent, session_topic, session_intent_source)
         VALUES ('s-memory', '/proj', 'ignored', ?1, 'doc', 'README navigation', 'override')",
        [SHANGHAI_NEW_YEAR],
    )
    .unwrap();

    let sessions = list_project(&conn, "/proj");
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].display_label.as_deref(),
        Some("0101｜doc｜README navigation")
    );
    assert_eq!(
        sessions[0].session_intent_source.as_deref(),
        Some("override")
    );
}
