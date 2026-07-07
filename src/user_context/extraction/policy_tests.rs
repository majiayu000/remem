use anyhow::Result;
use rusqlite::Connection;

use crate::db::{self, record_captured_event, CaptureEventInput, ExtractionTaskKind};

use super::*;

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    crate::migrate::run_migrations(&conn).expect("migrations should run");
    conn
}

fn capture_event(
    conn: &Connection,
    session_id: &str,
    role: Option<&str>,
    content: &str,
) -> Result<i64> {
    let outcome = record_captured_event(
        conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id,
            project: "/tmp/remem",
            cwd: None,
            event_type: "message",
            role,
            tool_name: None,
            content,
            task_kind: Some(ExtractionTaskKind::UserContextCandidate),
        },
    )?;
    Ok(outcome.event_row_id)
}

fn claim_task(conn: &mut Connection) -> Result<db::ExtractionTask> {
    db::claim_next_extraction_task(conn, "worker-a", 60)?
        .ok_or_else(|| anyhow::anyhow!("expected user-context candidate task"))
}

fn candidate_json(
    claim_type: &str,
    claim_key: &str,
    text: &str,
    confidence: f64,
    source_event_ids: &[i64],
) -> String {
    serde_json::json!({
        "candidates": [{
            "claim_type": claim_type,
            "claim_key": claim_key,
            "claim_text": text,
            "confidence": confidence,
            "sensitivity": "normal",
            "risk_class": "low",
            "source_kind": "explicit_user_statement",
            "source_event_ids": source_event_ids,
        }]
    })
    .to_string()
}

#[tokio::test]
async fn strict_auto_promote_policy_keeps_relaxed_default_fixture_pending() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-auto-strict",
        Some("user"),
        "I prefer concise code reviews.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator_strict(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:review-style",
            "User prefers concise code reviews.",
            0.75,
            &[event_id],
        ))
    })
    .await?;

    assert_eq!(
        result,
        UserContextCandidateExtractResult::Written {
            candidates: 1,
            promoted: 0,
            pending_review: 1,
            to_event_id: event_id,
        }
    );
    let (status, reason): (String, Option<String>) = conn.query_row(
        "SELECT review_status, auto_promote_block_reason FROM user_context_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(status, "pending_review");
    assert_eq!(reason.as_deref(), Some("low_confidence"));
    let active_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM user_context_claims WHERE status = 'active'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(active_count, 0);
    Ok(())
}
