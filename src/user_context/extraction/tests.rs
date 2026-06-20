use anyhow::Result;
use rusqlite::{params, Connection};

use crate::db::{self, record_captured_event, CaptureEventInput, ExtractionTaskKind};
use crate::user_context::claims::{
    ManualClaimRequest, UserContextClaimType, UserContextSensitivity,
};

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
    sensitivity: &str,
    risk_class: &str,
    source_kind: &str,
    source_event_ids: &[i64],
) -> String {
    serde_json::json!({
        "candidates": [{
            "claim_type": claim_type,
            "claim_key": claim_key,
            "claim_text": text,
            "confidence": confidence,
            "sensitivity": sensitivity,
            "risk_class": risk_class,
            "source_kind": source_kind,
            "source_event_ids": source_event_ids,
        }]
    })
    .to_string()
}

#[test]
fn parses_valid_and_no_candidate_responses() -> Result<()> {
    let parsed = parse_user_context_candidate_response(&candidate_json(
        "preference",
        "preference:review-style",
        "User prefers concise code reviews.",
        0.91,
        "normal",
        "low",
        "explicit_user_statement",
        &[1],
    ))?;
    match parsed {
        parse::UserContextCandidateResponse::Candidates(candidates) => {
            assert_eq!(candidates.len(), 1);
            assert_eq!(candidates[0].claim_type, UserContextClaimType::Preference);
            assert_eq!(candidates[0].claim_key, "preference:review-style");
            assert_eq!(candidates[0].source_event_ids, vec![1]);
        }
        parse::UserContextCandidateResponse::NoCandidates => panic!("expected candidates"),
    }
    assert_eq!(
        parse_user_context_candidate_response(
            r#"{"no_candidates":{"reason":"no stable user context"}}"#
        )?,
        parse::UserContextCandidateResponse::NoCandidates
    );
    Ok(())
}

#[test]
fn malformed_output_fails_closed() {
    let missing_refs = parse_user_context_candidate_response(
        r#"{"candidates":[{"claim_type":"preference","claim_key":"preference:review","claim_text":"missing refs","confidence":0.9,"sensitivity":"normal","risk_class":"low","source_kind":"explicit_user_statement"}]}"#,
    )
    .expect_err("missing source_event_ids must fail");
    assert!(format!("{missing_refs:#}").contains("missing field `source_event_ids`"));

    let empty_refs = parse_user_context_candidate_response(
        r#"{"candidates":[{"claim_type":"preference","claim_key":"preference:review","claim_text":"empty refs","confidence":0.9,"sensitivity":"normal","risk_class":"low","source_kind":"explicit_user_statement","source_event_ids":[]}]}"#,
    )
    .expect_err("empty source_event_ids must fail");
    assert!(empty_refs
        .to_string()
        .contains("source_event_ids must not be empty"));
}

#[tokio::test]
async fn low_risk_user_event_auto_promotes_to_active_claim() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-auto",
        Some("user"),
        "I prefer concise code reviews.",
    )?;
    let task = claim_task(&mut conn)?;
    insert_summary_for_task(&conn, &task)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:review-style",
            "User prefers concise code reviews.",
            0.93,
            "normal",
            "low",
            "explicit_user_statement",
            &[event_id],
        ))
    })
    .await?;

    assert_eq!(
        result,
        UserContextCandidateExtractResult::Written {
            candidates: 1,
            promoted: 1,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    let (status, source_refs): (String, String) = conn.query_row(
        "SELECT review_status, source_refs_json FROM user_context_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(status, "auto_promoted");
    assert!(source_refs.contains("captured_event"));
    assert!(source_refs.contains("session_summary"));
    let active_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM user_context_claims WHERE status = 'active'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(active_count, 1);
    Ok(())
}

#[tokio::test]
async fn assistant_sourced_explicit_statement_stays_pending_review() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-assistant",
        Some("assistant"),
        "The user probably prefers concise reviews.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:review-style",
            "User prefers concise code reviews.",
            0.93,
            "normal",
            "low",
            "explicit_user_statement",
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
    assert_eq!(reason.as_deref(), Some("source_not_user_authored"));
    let claim_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_claims", [], |row| {
            row.get(0)
        })?;
    assert_eq!(claim_count, 0);
    Ok(())
}

#[tokio::test]
async fn unsupported_user_event_citation_stays_pending_review() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-unsupported",
        Some("user"),
        "I prefer concise code reviews.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:release-notes",
            "User prefers verbose release notes.",
            0.96,
            "normal",
            "low",
            "explicit_user_statement",
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
    assert_eq!(reason.as_deref(), Some("no_supporting_source_event"));
    let claim_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_claims", [], |row| {
            row.get(0)
        })?;
    assert_eq!(claim_count, 0);
    Ok(())
}

#[tokio::test]
async fn replayed_candidate_output_does_not_duplicate_candidate() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-replay",
        Some("user"),
        "I prefer concise code reviews.",
    )?;
    let task = claim_task(&mut conn)?;

    for _ in 0..2 {
        process_with_generator(&mut conn, &task, |_prompt| async move {
            Ok(candidate_json(
                "preference",
                "preference:review-style",
                "User prefers concise code reviews.",
                0.93,
                "normal",
                "low",
                "explicit_user_statement",
                &[event_id],
            ))
        })
        .await?;
    }

    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    let active_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM user_context_claims WHERE status = 'active'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(candidate_count, 1);
    assert_eq!(active_count, 1);
    Ok(())
}

#[tokio::test]
async fn speculative_candidate_remains_pending_review() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-speculative",
        Some("user"),
        "I have been reading Rust compiler docs.",
    )?;
    let task = claim_task(&mut conn)?;

    process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "skill",
            "skill:rust",
            "User may know Rust.",
            0.67,
            "normal",
            "medium",
            "speculative_inference",
            &[event_id],
        ))
    })
    .await?;

    let (status, reason): (String, Option<String>) = conn.query_row(
        "SELECT review_status, auto_promote_block_reason FROM user_context_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(status, "pending_review");
    assert_eq!(reason.as_deref(), Some("claim_type_requires_review"));
    Ok(())
}

#[tokio::test]
async fn malformed_model_output_creates_no_candidate_or_claim() -> Result<()> {
    let mut conn = setup_conn();
    capture_event(
        &conn,
        "sess-user-context-malformed",
        Some("user"),
        "I prefer concise code reviews.",
    )?;
    let task = claim_task(&mut conn)?;

    let err = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(r#"{"candidates":[]}"#.to_string())
    })
    .await
    .expect_err("empty candidates must fail closed");

    assert!(err.to_string().contains("candidates must not be empty"));
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    let claim_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_claims", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    assert_eq!(claim_count, 0);
    Ok(())
}

#[tokio::test]
async fn source_ids_outside_loaded_range_fail_before_write() -> Result<()> {
    let mut conn = setup_conn();
    capture_event(
        &conn,
        "sess-user-context-bad-source",
        Some("user"),
        "I prefer concise code reviews.",
    )?;
    let task = claim_task(&mut conn)?;
    let missing_event = task.high_watermark_event_id.unwrap_or(0) + 100;

    let err = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:review-style",
            "User prefers concise code reviews.",
            0.93,
            "normal",
            "low",
            "explicit_user_statement",
            &[missing_event],
        ))
    })
    .await
    .expect_err("missing cited event should fail");

    assert!(err.to_string().contains("outside loaded source range"));
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[tokio::test]
async fn contradictory_candidate_supersedes_existing_claim_by_stable_key() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-supersede",
        Some("user"),
        "I now prefer detailed review summaries.",
    )?;
    let task = claim_task(&mut conn)?;
    super::super::claims::create_manual_claim(
        &conn,
        &ManualClaimRequest {
            text: "User prefers concise code reviews.",
            owner_scope: None,
            owner_key: None,
            claim_type: UserContextClaimType::Preference,
            claim_key: Some("preference:review-style"),
            confidence: 1.0,
            sensitivity: UserContextSensitivity::Normal,
            valid_from_epoch: None,
            valid_to_epoch: None,
        },
    )?;

    process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:review-style",
            "User prefers detailed review summaries.",
            0.94,
            "normal",
            "low",
            "explicit_user_statement",
            &[event_id],
        ))
    })
    .await?;

    let active_text: String = conn.query_row(
        "SELECT claim_text FROM user_context_claims WHERE status = 'active'",
        [],
        |row| row.get(0),
    )?;
    let superseded_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM user_context_claims WHERE status = 'superseded'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(active_text, "User prefers detailed review summaries.");
    assert_eq!(superseded_count, 1);
    Ok(())
}

fn insert_summary_for_task(conn: &Connection, task: &db::ExtractionTask) -> Result<()> {
    let session_row_id = task.session_row_id.expect("task should have session row");
    let to_event_id = task
        .high_watermark_event_id
        .expect("task should have watermark");
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, created_at, created_at_epoch,
          discovery_tokens, host_id, project_id, session_row_id, summary_text,
          covered_from_event_id, covered_to_event_id)
         VALUES (?1, ?2, 'Captured event range', 'User preference summary', '2026-06-20T00:00:00Z',
                 1782000000, 4, ?3, ?4, ?5, 'User prefers concise code reviews.', ?6, ?7)",
        params![
            "capture-rollup-test",
            task.project,
            task.host_id,
            task.project_id,
            session_row_id,
            to_event_id,
            to_event_id
        ],
    )?;
    Ok(())
}
