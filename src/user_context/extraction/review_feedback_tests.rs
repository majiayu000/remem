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
    db::claim_next_extraction_task(conn, "worker-review-feedback", 60)?
        .ok_or_else(|| anyhow::anyhow!("expected user-context candidate task"))
}

fn candidate_json(
    claim_type: &str,
    claim_key: &str,
    text: &str,
    confidence: f64,
    source_kind: &str,
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
            "source_kind": source_kind,
            "source_event_ids": source_event_ids,
        }]
    })
    .to_string()
}

fn candidate_count(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
        row.get(0)
    })
    .map_err(Into::into)
}

#[tokio::test]
async fn according_to_readme_blocks_unapproved_external_candidate() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-review-readme-attribution",
        Some("user"),
        "According to the README, the user works on internal payroll.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "project",
            "project:payroll",
            "User works on internal payroll.",
            0.93,
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
    assert_eq!(candidate_count(&conn)?, 0);
    Ok(())
}

#[tokio::test]
async fn readme_approval_does_not_cover_generic_file_evidence() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-review-readme-file-mismatch",
        Some("user"),
        "Please remember from README that I work on remem. File says the user works on payroll.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "project",
            "project:payroll",
            "User works on payroll.",
            0.93,
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
    assert_eq!(candidate_count(&conn)?, 0);
    Ok(())
}

#[tokio::test]
async fn lowercase_third_party_subject_does_not_auto_promote() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-review-lowercase-third-party",
        Some("user"),
        "Alice prefers concise reviews.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:alice-review-style",
            "alice prefers concise reviews.",
            0.92,
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
    assert_eq!(candidate_count(&conn)?, 0);
    Ok(())
}

#[tokio::test]
async fn incidental_general_fact_does_not_drop_valid_preference() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-review-incidental-general-fact",
        Some("user"),
        "I prefer Rust for scripts because Rust ownership prevents data races.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:rust",
            "User prefers Rust for scripts.",
            0.93,
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
    assert_eq!(candidate_count(&conn)?, 1);
    Ok(())
}

#[tokio::test]
async fn ordinary_web_page_preference_is_not_external_source() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-review-web-page-preference",
        Some("user"),
        "I prefer testing web page layouts in Playwright.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:web-page-layout-testing",
            "User prefers testing web page layouts in Playwright.",
            0.93,
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
    assert_eq!(candidate_count(&conn)?, 1);
    Ok(())
}

#[tokio::test]
async fn unapproved_web_page_attribution_creates_no_candidate() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-review-web-page-attribution",
        Some("user"),
        "From the web page, the user lives in Paris.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "identity",
            "identity:location",
            "User lives in Paris.",
            0.93,
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
    assert_eq!(candidate_count(&conn)?, 0);
    Ok(())
}

#[tokio::test]
async fn lowercase_tool_subject_preference_is_not_third_party() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-review-lowercase-tool-subject",
        Some("user"),
        "Playwright is my preferred layout test runner.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:layout-test-runner",
            "playwright is the user's preferred layout test runner.",
            0.93,
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
    assert_eq!(candidate_count(&conn)?, 1);
    Ok(())
}

#[tokio::test]
async fn user_framed_general_technical_fact_creates_no_candidate() -> Result<()> {
    let mut conn = setup_conn();
    let event_id = capture_event(
        &conn,
        "sess-review-user-framed-general-fact",
        Some("user"),
        "I think SQLite is a single-file database.",
    )?;
    let task = claim_task(&mut conn)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(candidate_json(
            "preference",
            "preference:sqlite-fact",
            "User thinks SQLite is a single-file database.",
            0.93,
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
    assert_eq!(candidate_count(&conn)?, 0);
    Ok(())
}
