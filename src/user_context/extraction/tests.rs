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
    capture_event_with_details(conn, session_id, "message", role, None, content)
}

fn capture_event_with_details(
    conn: &Connection,
    session_id: &str,
    event_type: &str,
    role: Option<&str>,
    tool_name: Option<&str>,
    content: &str,
) -> Result<i64> {
    let outcome = record_captured_event(
        conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id,
            project: "/tmp/remem",
            cwd: None,
            event_type,
            role,
            tool_name,
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

#[test]
fn prompt_json_contains_non_retention_blocklist() -> Result<()> {
    let mut conn = setup_conn();
    capture_event(
        &conn,
        "sess-user-context-prompt-policy",
        Some("user"),
        "I prefer concise code reviews.",
    )?;
    let task = claim_task(&mut conn)?;
    let batch = source::load_source_batch(&conn, &task)?.expect("source batch should load");

    let prompt = prompt::build_candidate_prompt(&task, &batch)?;
    let value: serde_json::Value = serde_json::from_str(&prompt)?;
    let policy = value["non_retention_policy"]
        .as_array()
        .expect("prompt should include non_retention_policy");

    for expected in prompt::NON_RETENTION_POLICY {
        assert!(
            policy.iter().any(|item| item.as_str() == Some(*expected)),
            "missing non-retention rule: {expected}"
        );
    }
    Ok(())
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
async fn source_whitespace_normalization_does_not_trigger_secret_block() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-whitespace",
        Some("user"),
        "I prefer   concise\tcode reviews.",
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
            promoted: 1,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 1);
    Ok(())
}

#[tokio::test]
async fn dotted_version_evidence_stays_in_one_segment() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-dotted-version",
        Some("user"),
        "I prefer Python 3.11 for scripts.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:python-version",
            "User prefers Python 3.11 for scripts.",
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
    Ok(())
}

#[tokio::test]
async fn task_specific_low_risk_words_do_not_trigger_secret_block() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-secret-prefix-false-positive",
        Some("user"),
        "I prefer task-specific low-risk code reviews.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:task-specific-low-risk-reviews",
            "User prefers task-specific low-risk code reviews.",
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
    Ok(())
}

#[tokio::test]
async fn summary_blocklist_text_does_not_block_direct_candidate_evidence() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-direct-evidence",
        Some("user"),
        "I prefer concise code reviews.",
    )?;
    let task = claim_task(&mut conn)?;
    insert_summary_for_task_with_text(
        &conn,
        &task,
        "Unrelated range note: user is tired and mentioned an API key.",
    )?;

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
    let source_preview: String = conn.query_row(
        "SELECT source_preview FROM user_context_candidates",
        [],
        |row| row.get(0),
    )?;
    assert!(source_preview.contains("I prefer concise code reviews."));
    assert!(!source_preview.contains("API key"));
    Ok(())
}

#[tokio::test]
async fn source_preview_trims_unrelated_third_party_detail() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-preview-trim",
        Some("user"),
        "I prefer concise code reviews. Alice lives in Boston.",
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
            promoted: 1,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    let source_preview: String = conn.query_row(
        "SELECT source_preview FROM user_context_candidates",
        [],
        |row| row.get(0),
    )?;
    assert!(source_preview.contains("I prefer concise code reviews."));
    assert!(!source_preview.contains("Alice lives in Boston"));
    Ok(())
}

#[tokio::test]
async fn source_preview_requires_user_subject_for_user_claims() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-preview-user-subject",
        Some("user"),
        "I prefer concise reviews. Alice prefers concise reviews in Boston.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:review-style",
            "User prefers concise reviews.",
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
    let source_preview: String = conn.query_row(
        "SELECT source_preview FROM user_context_candidates",
        [],
        |row| row.get(0),
    )?;
    assert!(source_preview.contains("I prefer concise reviews."));
    assert!(!source_preview.contains("Alice prefers concise reviews"));
    Ok(())
}

#[tokio::test]
async fn non_retention_scan_uses_matched_source_preview_only() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-preview-blocklist-scope",
        Some("user"),
        "I prefer concise reviews. I had sushi for lunch today.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:review-style",
            "User prefers concise reviews.",
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
    let source_preview: String = conn.query_row(
        "SELECT source_preview FROM user_context_candidates",
        [],
        |row| row.get(0),
    )?;
    assert!(source_preview.contains("I prefer concise reviews."));
    assert!(!source_preview.contains("sushi"));
    Ok(())
}

#[tokio::test]
async fn source_preview_preserves_external_source_approval_context() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-readme-approval",
        Some("user"),
        "I work on remem from README. Please remember from README.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "project",
            "project:remem-readme",
            "User works on remem from README.",
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
    let (source_preview, reason): (String, Option<String>) = conn.query_row(
        "SELECT source_preview, auto_promote_block_reason FROM user_context_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert!(source_preview.contains("Please remember from README."));
    assert!(source_preview.contains("I work on remem from README."));
    assert_eq!(reason.as_deref(), Some("claim_type_requires_review"));
    Ok(())
}

#[tokio::test]
async fn external_source_approval_must_match_evidence_source() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-source-approval-mismatch",
        Some("user"),
        "Please remember from README that I work on remem. The website says the user lives in Paris.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "identity",
            "identity:location",
            "User lives in Paris.",
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
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[tokio::test]
async fn website_source_approval_allows_review_candidate() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-website-approval",
        Some("user"),
        "The website says the user lives in Paris. Please remember from website.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "identity",
            "identity:location",
            "User lives in Paris.",
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
    let source_preview: String = conn.query_row(
        "SELECT source_preview FROM user_context_candidates",
        [],
        |row| row.get(0),
    )?;
    assert!(source_preview.contains("Please remember from website."));
    assert!(source_preview.contains("website says the user lives in Paris"));
    Ok(())
}

#[tokio::test]
async fn assistant_sourced_explicit_statement_creates_no_candidate() -> Result<()> {
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
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    let claim_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_claims", [], |row| {
            row.get(0)
        })?;
    assert_eq!(claim_count, 0);
    Ok(())
}

#[tokio::test]
async fn mixed_assistant_claim_with_unrelated_user_source_creates_no_candidate() -> Result<()> {
    let mut conn = setup_conn();
    let user_event_id = capture_event(
        &conn,
        "sess-user-context-mixed-source",
        Some("user"),
        "I prefer concise code reviews.",
    )?;
    let assistant_event_id = capture_event(
        &conn,
        "sess-user-context-mixed-source",
        Some("assistant"),
        "The user prefers verbose release notes.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:release-notes",
            "User prefers verbose release notes.",
            0.93,
            "normal",
            "low",
            "session_summary",
            &[user_event_id, assistant_event_id],
        ))
    })
    .await?;

    assert_eq!(
        result,
        UserContextCandidateExtractResult::Written {
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: assistant_event_id,
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[tokio::test]
async fn secret_like_candidate_output_creates_no_candidate() -> Result<()> {
    blocked_candidate_creates_no_rows(
        Some("user"),
        "My API key is sk-testsecret123456.",
        "User's API key is sk-testsecret123456.",
        "explicit_user_statement",
    )
    .await
}

#[tokio::test]
async fn account_number_candidate_output_creates_no_candidate() -> Result<()> {
    blocked_candidate_creates_no_rows(
        Some("user"),
        "My bank account number is 123456789.",
        "User's bank account number is 123456789.",
        "explicit_user_statement",
    )
    .await
}

#[tokio::test]
async fn roleplay_hypothetical_candidate_creates_no_candidate() -> Result<()> {
    blocked_candidate_creates_no_rows(
        Some("user"),
        "As a joke, pretend I am the CEO of Example Corp.",
        "User is hypothetically the CEO of Example Corp.",
        "explicit_user_statement",
    )
    .await
}

#[tokio::test]
async fn temporary_state_candidate_creates_no_candidate() -> Result<()> {
    blocked_candidate_creates_no_rows(
        Some("user"),
        "I am tired today after lunch.",
        "User is tired today after lunch.",
        "explicit_user_statement",
    )
    .await
}

#[tokio::test]
async fn general_technical_fact_creates_no_candidate() -> Result<()> {
    blocked_candidate_creates_no_rows(
        Some("user"),
        "Rust ownership prevents data races.",
        "Rust ownership prevents data races.",
        "explicit_user_statement",
    )
    .await
}

#[tokio::test]
async fn unapproved_file_derived_claim_creates_no_candidate() -> Result<()> {
    blocked_candidate_creates_no_rows(
        Some("assistant"),
        "From README files, the user works on internal payroll systems.",
        "User works on internal payroll systems from files without user approval.",
        "session_summary",
    )
    .await
}

#[tokio::test]
async fn user_framed_third_party_candidate_stays_pending_review() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-third-party",
        Some("user"),
        "My teammate Alice owns release QA for my remem workflow.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "relationship",
            "relationship:alice-release-qa",
            "Alice owns release QA for the user's remem workflow.",
            0.92,
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
    let (status, reason): (String, Option<String>) = conn.query_row(
        "SELECT review_status, auto_promote_block_reason FROM user_context_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(status, "pending_review");
    assert_eq!(reason.as_deref(), Some("third_party_requires_review"));
    let claim_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_claims", [], |row| {
            row.get(0)
        })?;
    assert_eq!(claim_count, 0);
    Ok(())
}

#[tokio::test]
async fn paraphrased_user_framed_third_party_relationship_stays_pending_review() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-third-party-paraphrase",
        Some("user"),
        "My manager is Alice.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "relationship",
            "relationship:alice-manager",
            "Alice is the user's manager.",
            0.92,
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
    Ok(())
}

#[tokio::test]
async fn user_framed_third_party_candidate_with_changed_fact_creates_no_candidate() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-third-party-changed-fact",
        Some("user"),
        "My manager is Alice.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "relationship",
            "relationship:bob-manager",
            "Bob is the user's manager.",
            0.92,
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
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[tokio::test]
async fn third_party_framing_must_share_evidence_segment() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-third-party-cross-sentence",
        Some("user"),
        "My manager is Alice. Bob owns release QA.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "relationship",
            "relationship:bob-release-qa",
            "Bob owns release QA.",
            0.92,
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
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[tokio::test]
async fn unframed_relationship_mislabeled_explicit_creates_no_candidate() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-mislabeled-third-party",
        Some("user"),
        "Alice lives in Boston.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "relationship",
            "relationship:alice-location",
            "Alice lives in Boston.",
            0.92,
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
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[tokio::test]
async fn negated_third_party_relationship_creates_no_candidate() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-negated-third-party",
        Some("user"),
        "My manager is not Alice.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "relationship",
            "relationship:alice-manager",
            "Alice is the user's manager.",
            0.92,
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
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[tokio::test]
async fn family_framed_third_party_relationship_stays_pending_review() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-family-third-party",
        Some("user"),
        "My wife is Alice.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "relationship",
            "relationship:alice-wife",
            "Alice is the user's wife.",
            0.92,
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
    let reason: Option<String> = conn.query_row(
        "SELECT auto_promote_block_reason FROM user_context_candidates",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(reason.as_deref(), Some("third_party_requires_review"));
    Ok(())
}

#[tokio::test]
async fn assistant_only_framed_third_party_detail_creates_no_candidate() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-third-party-assistant-only",
        Some("assistant"),
        "Alice owns release QA for the user's workflow.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "relationship",
            "relationship:alice-release-qa",
            "Alice owns release QA for the user's workflow.",
            0.92,
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
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    Ok(())
}

#[tokio::test]
async fn unframed_third_party_detail_creates_no_candidate() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-unframed-third-party",
        Some("user"),
        "Alice lives in Boston.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "relationship",
            "relationship:alice-location",
            "Alice lives in Boston.",
            0.92,
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
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[tokio::test]
async fn first_person_unframed_third_party_detail_creates_no_candidate() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-first-person-unframed-third-party",
        Some("user"),
        "I heard Alice lives in Boston.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "relationship",
            "relationship:alice-location",
            "Alice lives in Boston.",
            0.92,
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
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    Ok(())
}

#[tokio::test]
async fn unsupported_user_event_citation_creates_no_candidate() -> Result<()> {
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
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    let claim_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_claims", [], |row| {
            row.get(0)
        })?;
    assert_eq!(claim_count, 0);
    Ok(())
}

#[tokio::test]
async fn support_matching_does_not_combine_separate_sentences() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-sentence-boundary",
        Some("user"),
        "I prefer concise reviews. Verbose release notes are hard to scan.",
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
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[tokio::test]
async fn inferred_behavior_source_stays_pending_review() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event_with_details(
        &conn,
        "sess-user-context-behavior-source",
        "tool_result",
        None,
        Some("Bash"),
        "Ran cargo test for remem verification.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:remem-verification",
            "User prefers cargo test for remem verification.",
            0.93,
            "normal",
            "low",
            "inferred_from_behavior",
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
    assert_eq!(reason.as_deref(), Some("source_requires_review"));
    Ok(())
}

#[tokio::test]
async fn assistant_text_cannot_be_behavior_source() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-assistant-behavior-source",
        Some("assistant"),
        "The user prefers verbose release notes.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:release-notes",
            "User prefers verbose release notes.",
            0.93,
            "normal",
            "low",
            "inferred_from_behavior",
            &[event_id],
        ))
    })
    .await?;

    assert_eq!(
        result,
        UserContextCandidateExtractResult::Written {
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[tokio::test]
async fn mixed_assistant_text_cannot_borrow_behavior_source() -> Result<()> {
    let mut conn = setup_conn();
    let tool_event_id = capture_event_with_details(
        &conn,
        "sess-user-context-mixed-behavior-source",
        "tool_result",
        None,
        Some("Bash"),
        "Ran cargo test for remem verification.",
    )?;
    let assistant_event_id = capture_event(
        &conn,
        "sess-user-context-mixed-behavior-source",
        Some("assistant"),
        "The user prefers verbose release notes.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:release-notes",
            "User prefers verbose release notes.",
            0.93,
            "normal",
            "low",
            "inferred_from_behavior",
            &[tool_event_id, assistant_event_id],
        ))
    })
    .await?;

    assert_eq!(
        result,
        UserContextCandidateExtractResult::Written {
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: assistant_event_id,
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[tokio::test]
async fn explicit_negative_constraint_stays_pending_review() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-negative-constraint",
        Some("user"),
        "I never want auto-merge enabled.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "constraint",
            "constraint:auto-merge-disabled",
            "User never wants auto-merge enabled.",
            0.94,
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
    let reason: Option<String> = conn.query_row(
        "SELECT auto_promote_block_reason FROM user_context_candidates",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(reason.as_deref(), Some("no_supporting_source_event"));
    Ok(())
}

#[tokio::test]
async fn negative_constraint_fallback_does_not_cross_segments() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-negative-constraint-cross-segment",
        Some("user"),
        "I never want auto-merge. Enabled feature flags are okay.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "constraint",
            "constraint:auto-merge-enabled-disabled",
            "User never wants auto-merge enabled.",
            0.94,
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
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

async fn blocked_candidate_creates_no_rows(
    role: Option<&str>,
    event_content: &str,
    candidate_text: &str,
    source_kind: &str,
) -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(&conn, "sess-user-context-blocked", role, event_content)?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:blocked",
            candidate_text,
            0.95,
            "normal",
            "low",
            source_kind,
            &[event_id],
        ))
    })
    .await?;

    assert_eq!(
        result,
        UserContextCandidateExtractResult::Written {
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
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
async fn speculative_candidate_creates_no_candidate() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-user-context-speculative",
        Some("user"),
        "I have been reading Rust compiler docs.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
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

    assert_eq!(
        result,
        UserContextCandidateExtractResult::Written {
            candidates: 0,
            promoted: 0,
            pending_review: 0,
            to_event_id: event_id,
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
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

    let (candidate_id, status, reason): (i64, String, Option<String>) = conn.query_row(
        "SELECT id, review_status, auto_promote_block_reason FROM user_context_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
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

    super::super::candidates::approve_candidate(&conn, candidate_id)?;

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
    insert_summary_for_task_with_text(conn, task, "User prefers concise code reviews.")
}

fn insert_summary_for_task_with_text(
    conn: &Connection,
    task: &db::ExtractionTask,
    summary_text: &str,
) -> Result<()> {
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
                 1782000000, 4, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            "capture-rollup-test",
            task.project,
            task.host_id,
            task.project_id,
            session_row_id,
            summary_text,
            to_event_id,
            to_event_id
        ],
    )?;
    Ok(())
}
