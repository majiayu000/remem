use anyhow::Result;
use rusqlite::Connection;

use super::super::promote_summary_to_memory_candidates;
use super::super::slug::content_hash;
use crate::db;

fn setup_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

fn record_summary_evidence(conn: &Connection, session_id: &str, project: &str) -> Result<i64> {
    let outcome = db::record_captured_event(
        conn,
        &db::CaptureEventInput {
            host: "codex-cli",
            session_id,
            project,
            cwd: Some(project),
            event_type: "session_stop",
            role: None,
            tool_name: None,
            content: "summary source payload",
            task_kind: Some(db::ExtractionTaskKind::SessionRollup),
        },
    )?;
    Ok(outcome.event_row_id)
}

#[test]
fn test_summary_candidates_multi_decisions_do_not_create_memories() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-decisions";
    let project = "test/proj";
    let evidence_id = record_summary_evidence(&conn, session_id, project)?;

    let decisions = "• Use RwLock instead of Mutex for concurrent read support\n\
                     • Switch to trigram tokenizer for CJK text search\n\
                     • Set compression threshold to 100 observations";
    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Optimize search and concurrency"),
        Some(decisions),
        None,
        None,
    )?;
    assert_eq!(count, 3);

    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    let candidate_rows = conn
        .prepare(
            "SELECT memory_type, review_status, evidence_event_ids
             FROM memory_candidates
             ORDER BY id ASC",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let evidence_json = serde_json::to_string(&vec![evidence_id])?;

    assert_eq!(memory_count, 0);
    assert_eq!(candidate_rows.len(), 3);
    assert!(candidate_rows
        .iter()
        .all(|row| row.0 == "decision" && row.1 == "pending_review" && row.2 == evidence_json));
    Ok(())
}

#[test]
fn test_summary_candidates_learned_lesson_and_discovery() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-learned";
    let project = "test/proj";
    record_summary_evidence(&conn, session_id, project)?;

    let learned = "- FTS5 trigram tokenizer handles CJK without word boundaries\n\
                   - Root cause: warning-only fallback hid missing data; avoid silent degradation.";
    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Research storage"),
        None,
        Some(learned),
        None,
    )?;
    assert_eq!(count, 2);

    let rows = conn
        .prepare(
            "SELECT memory_type, confidence, review_status
             FROM memory_candidates
             ORDER BY memory_type ASC",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "discovery");
    assert_eq!(rows[0].2, "pending_review");
    assert_eq!(rows[1].0, "lesson");
    assert!(rows[1].1 >= 0.8);
    assert_eq!(rows[1].2, "pending_review");
    Ok(())
}

#[test]
fn test_summary_candidate_duplicate_output_is_idempotent() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-dup";
    let project = "test/proj";
    record_summary_evidence(&conn, session_id, project)?;

    let decision = "Use FTS5 trigram tokenizer for CJK text search support";
    let first = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Optimize search"),
        Some(decision),
        None,
        None,
    )?;
    let second = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Optimize search"),
        Some(decision),
        None,
        None,
    )?;

    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(first, 1);
    assert_eq!(second, 0);
    assert_eq!(candidate_count, 1);
    Ok(())
}

#[test]
fn test_summary_preference_candidate_defaults_to_project_scope() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-preference";
    let project = "test/proj";
    record_summary_evidence(&conn, session_id, project)?;

    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Capture local preference"),
        None,
        None,
        Some("Always run project-specific smoke tests"),
    )?;
    assert_eq!(count, 1);

    let (scope, memory_type, owner_scope, owner_key): (String, String, String, String) = conn
        .query_row(
            "SELECT scope, memory_type, owner_scope, owner_key FROM memory_candidates",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    assert_eq!(scope, "project");
    assert_eq!(memory_type, "preference");
    assert_eq!(owner_scope, "repo");
    assert_eq!(owner_key, project);
    Ok(())
}

#[test]
fn test_summary_candidates_missing_evidence_fails_closed() -> Result<()> {
    let mut conn = setup_conn()?;

    let err = promote_summary_to_memory_candidates(
        &mut conn,
        "missing-evidence",
        "test/proj",
        Some("Optimize search"),
        Some("Use FTS5 trigram tokenizer for CJK text search support"),
        None,
        None,
    )
    .expect_err("missing captured evidence should fail closed");

    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    assert!(err.to_string().contains("missing captured evidence"));
    assert_eq!(memory_count, 0);
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[test]
fn test_summary_candidate_content_keeps_compact_context() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-content";
    let project = "test/proj";
    record_summary_evidence(&conn, session_id, project)?;

    promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Fix search"),
        Some("Switched from unicode61 to trigram tokenizer for better CJK support"),
        None,
        None,
    )?;

    let text: String =
        conn.query_row("SELECT text FROM memory_candidates", [], |row| row.get(0))?;
    assert!(
        !text.contains("**Request**"),
        "content should not have boilerplate: {text}"
    );
    assert!(
        text.contains("[Context:"),
        "content should have compact context: {text}"
    );
    Ok(())
}

#[test]
fn test_content_hash_dedup() {
    let hash1 = content_hash("Use FTS5 trigram tokenizer for CJK support");
    let hash2 = content_hash("Use FTS5 trigram tokenizer for CJK support");
    assert_eq!(hash1, hash2);

    let hash3 = content_hash("Switch to WAL mode for concurrent reads");
    assert_ne!(hash1, hash3);
}
