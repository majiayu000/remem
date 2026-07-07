use anyhow::Result;
use rusqlite::Connection;

use crate::db::{self, record_captured_event, CaptureEventInput, ExtractionTaskKind};
use crate::user_context::claims::{
    create_manual_claim, ManualClaimRequest, UserContextClaimType, UserContextSensitivity,
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
    source_event_ids: &[i64],
) -> String {
    candidate_json_with(
        claim_type,
        claim_key,
        text,
        confidence,
        "normal",
        "low",
        "explicit_user_statement",
        source_event_ids,
    )
}

fn candidate_json_with(
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

fn candidate_status_and_reason(conn: &Connection) -> Result<(String, Option<String>)> {
    conn.query_row(
        "SELECT review_status, auto_promote_block_reason FROM user_context_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(Into::into)
}

fn active_claim_count(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM user_context_claims WHERE status = 'active'",
        [],
        |row| row.get(0),
    )
    .map_err(Into::into)
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

#[tokio::test]
async fn relaxed_policy_keeps_sensitivity_and_risk_review_gated() -> Result<()> {
    for (label, sensitivity, risk_class, expected_reason) in [
        (
            "sensitive",
            "sensitive",
            "low",
            "sensitivity_requires_review",
        ),
        ("high-risk", "normal", "high", "risk_requires_review"),
    ] {
        let mut conn = setup_conn();
        let event_id = capture_event(
            &conn,
            &format!("sess-user-context-relaxed-{label}"),
            Some("user"),
            "I prefer concise code reviews.",
        )?;
        let task = claim_task(&mut conn)?;
        let response = candidate_json_with(
            "preference",
            "preference:review-style",
            "User prefers concise code reviews.",
            0.75,
            sensitivity,
            risk_class,
            "explicit_user_statement",
            &[event_id],
        );

        let result =
            process_with_generator(&mut conn, &task, |_prompt| async move { Ok(response) }).await?;

        assert_eq!(
            result,
            UserContextCandidateExtractResult::Written {
                candidates: 1,
                promoted: 0,
                pending_review: 1,
                to_event_id: event_id,
            }
        );
        let (status, reason) = candidate_status_and_reason(&conn)?;
        assert_eq!(status, "pending_review");
        assert_eq!(reason.as_deref(), Some(expected_reason), "{label}");
        assert_eq!(active_claim_count(&conn)?, 0);
    }
    Ok(())
}

#[tokio::test]
async fn relaxed_policy_keeps_third_party_framing_review_gated() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-relaxed-third-party",
        Some("user"),
        "My teammate Alice owns release QA for my remem workflow.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json_with(
            "relationship",
            "relationship:alice-release-qa",
            "Alice owns release QA for the user's remem workflow.",
            0.75,
            "normal",
            "low",
            "third_party_statement",
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
    let (status, reason) = candidate_status_and_reason(&conn)?;
    assert_eq!(status, "pending_review");
    assert_eq!(reason.as_deref(), Some("third_party_requires_review"));
    assert_eq!(active_claim_count(&conn)?, 0);
    Ok(())
}

#[tokio::test]
async fn relaxed_policy_drops_non_user_sources_and_non_retention_before_candidate() -> Result<()> {
    for (label, role, source_text, candidate_text) in [
        (
            "assistant-source",
            Some("assistant"),
            "The user prefers concise code reviews.",
            "User prefers concise code reviews.",
        ),
        (
            "secret",
            Some("user"),
            "My API key is sk-testsecret123456.",
            "User's API key is sk-testsecret123456.",
        ),
    ] {
        let mut conn = setup_conn();
        let event_id = capture_event(
            &conn,
            &format!("sess-user-context-relaxed-{label}"),
            role,
            source_text,
        )?;
        let task = claim_task(&mut conn)?;
        let response = candidate_json(
            "preference",
            "preference:review-style",
            candidate_text,
            0.75,
            &[event_id],
        );

        let result =
            process_with_generator(&mut conn, &task, |_prompt| async move { Ok(response) }).await?;

        assert_eq!(
            result,
            UserContextCandidateExtractResult::Written {
                candidates: 0,
                promoted: 0,
                pending_review: 0,
                to_event_id: event_id,
            },
            "{label}"
        );
        let candidate_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
                row.get(0)
            })?;
        assert_eq!(candidate_count, 0, "{label}");
        assert_eq!(active_claim_count(&conn)?, 0, "{label}");
    }
    Ok(())
}

#[tokio::test]
async fn relaxed_policy_keeps_claim_key_conflicts_pending_review() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-relaxed-claim-key-conflict",
        Some("user"),
        "I now prefer detailed review summaries.",
    )?;
    let task = claim_task(&mut conn)?;
    create_manual_claim(
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

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:review-style",
            "User prefers detailed review summaries.",
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
    let (status, reason) = candidate_status_and_reason(&conn)?;
    assert_eq!(status, "pending_review");
    assert_eq!(
        reason.as_deref(),
        Some("claim_key_conflict_requires_review")
    );
    let active_text: String = conn.query_row(
        "SELECT claim_text FROM user_context_claims WHERE status = 'active'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(active_text, "User prefers concise code reviews.");
    Ok(())
}
