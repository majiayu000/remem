use axum::{
    body::{to_bytes, Body},
    http::{Method, Request, StatusCode},
};
use rusqlite::params;
use serde_json::{json, Value};
use std::sync::{Arc, Barrier};
use tower::ServiceExt;

use crate::api::DbState;
use crate::db;
use crate::db::test_support::ScopedTestDataDir;

use super::super::handlers::execute_safe_review_for_test;
use super::{authorized_json_request, insert_safe_review_candidate};

mod stable_errors;

fn candidate_version(id: i64) -> anyhow::Result<i64> {
    let conn = db::open_db()?;
    conn.query_row(
        "SELECT version FROM memory_candidates WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

async fn response_json(response: axum::response::Response) -> anyhow::Result<Value> {
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&body)?)
}

async fn send_safe_review(
    candidate_id: i64,
    action: &str,
    token: &str,
    body: Value,
) -> anyhow::Result<axum::response::Response> {
    let app = super::super::build_router(0).with_state(DbState);
    app.oneshot(authorized_json_request(
        Method::POST,
        &format!("/api/v1/candidates/{candidate_id}/review/{action}"),
        token,
        &serde_json::to_string(&body)?,
    ))
    .await
    .map_err(Into::into)
}

#[tokio::test]
async fn safe_approve_is_atomic_audited_and_replayable() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-approve-replay");
    let (candidate_id, _) = insert_safe_review_candidate(
        "safe-approve-replay",
        "file_edit",
        "raw evidence must not enter the response",
        "Use an immediate transaction for review.",
    )?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let body = json!({
        "reason": "Reviewed against source evidence",
        "expected_version": candidate_version(candidate_id)?,
        "idempotency_key": "safe-approve-replay-1"
    });

    let response = send_safe_review(candidate_id, "approve", &token, body.clone()).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let first = response_json(response).await?;
    assert_eq!(first["candidate_id"], candidate_id);
    assert_eq!(first["action"], "approve");
    assert_eq!(first["before_status"], "pending_review");
    assert_eq!(first["after_status"], "approved");
    assert_eq!(first["replayed"], false);
    assert!(first["memory_id"].as_i64().is_some());

    let response = send_safe_review(candidate_id, "approve", &token, body).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let replay = response_json(response).await?;
    assert_eq!(replay["replayed"], true);
    for field in ["operation_id", "audit_id", "memory_id", "version"] {
        assert_eq!(replay[field], first[field]);
    }

    let conn = db::open_db()?;
    let audit_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE event_type = 'candidate_review'",
        [],
        |row| row.get(0),
    )?;
    let ledger_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM api_mutation_requests", [], |row| {
            row.get(0)
        })?;
    assert_eq!(audit_count, 1);
    assert_eq!(ledger_count, 1);
    let persisted: String = conn.query_row(
        "SELECT response_json || idempotency_key_hash FROM api_mutation_requests LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    assert!(!persisted.contains("safe-approve-replay-1"));
    Ok(())
}

#[tokio::test]
async fn idempotency_conflict_precedes_changed_candidate_state() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-review-key-conflict");
    let (candidate_id, _) = insert_safe_review_candidate(
        "safe-review-key-conflict",
        "file_edit",
        "safe evidence",
        "Candidate for conflict ordering.",
    )?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let version = candidate_version(candidate_id)?;
    let approve = json!({
        "reason": "approve once",
        "expected_version": version,
        "idempotency_key": "same-key-different-body"
    });
    assert_eq!(
        send_safe_review(candidate_id, "approve", &token, approve)
            .await?
            .status(),
        StatusCode::OK
    );

    let conflict = send_safe_review(
        candidate_id,
        "reject",
        &token,
        json!({
            "reason": "different action after state changed",
            "expected_version": version,
            "idempotency_key": "same-key-different-body"
        }),
    )
    .await?;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let payload = response_json(conflict).await?;
    assert_eq!(payload["error"]["code"], "idempotency_conflict");
    Ok(())
}

#[tokio::test]
async fn safe_review_rejects_stale_version_without_side_effects() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-review-stale");
    let (candidate_id, _) = insert_safe_review_candidate(
        "safe-review-stale",
        "file_edit",
        "safe evidence",
        "Candidate for version conflict.",
    )?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let response = send_safe_review(
        candidate_id,
        "reject",
        &token,
        json!({
            "reason": "stale client",
            "expected_version": candidate_version(candidate_id)? + 1,
            "idempotency_key": "stale-review-1"
        }),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let payload = response_json(response).await?;
    assert_eq!(payload["error"]["code"], "version_conflict");

    let conn = db::open_db()?;
    let status: String = conn.query_row(
        "SELECT review_status FROM memory_candidates WHERE id = ?1",
        params![candidate_id],
        |row| row.get(0),
    )?;
    let audit_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE event_type = 'candidate_review'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(status, "pending_review");
    assert_eq!(audit_count, 0);
    Ok(())
}

#[tokio::test]
async fn safe_reject_and_edit_use_the_new_envelope() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-reject-edit");
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let (reject_id, _) = insert_safe_review_candidate(
        "safe-reject",
        "file_edit",
        "safe evidence",
        "Candidate to reject.",
    )?;
    let rejected = send_safe_review(
        reject_id,
        "reject",
        &token,
        json!({
            "reason": "not durable",
            "expected_version": candidate_version(reject_id)?,
            "idempotency_key": "safe-reject-1"
        }),
    )
    .await?;
    assert_eq!(rejected.status(), StatusCode::OK);
    let rejected = response_json(rejected).await?;
    assert_eq!(rejected["action"], "reject");
    assert_eq!(rejected["after_status"], "discarded");
    assert_eq!(rejected["memory_id"], Value::Null);

    let (edit_id, _) = insert_safe_review_candidate(
        "safe-edit",
        "file_edit",
        "safe evidence",
        "Candidate to edit.",
    )?;
    let edited = send_safe_review(
        edit_id,
        "edit",
        &token,
        json!({
            "reason": "clarify durable text",
            "expected_version": candidate_version(edit_id)?,
            "idempotency_key": "safe-edit-1",
            "text": "Use checked transactions for console mutations."
        }),
    )
    .await?;
    assert_eq!(edited.status(), StatusCode::OK);
    let edited = response_json(edited).await?;
    assert_eq!(edited["action"], "edit");
    assert_eq!(edited["after_status"], "edited");
    assert!(edited["memory_id"].as_i64().is_some());
    Ok(())
}

#[tokio::test]
async fn audit_failure_rolls_back_candidate_memory_and_ledger() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-review-rollback");
    let (candidate_id, _) = insert_safe_review_candidate(
        "safe-review-rollback",
        "file_edit",
        "safe evidence",
        "Candidate whose audit insert fails.",
    )?;
    let conn = db::open_db()?;
    conn.execute_batch(
        "CREATE TRIGGER reject_candidate_review_audit
         BEFORE INSERT ON events
         WHEN NEW.event_type = 'candidate_review'
         BEGIN SELECT RAISE(ABORT, 'forced audit failure'); END;",
    )?;
    drop(conn);
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let response = send_safe_review(
        candidate_id,
        "approve",
        &token,
        json!({
            "reason": "should roll back",
            "expected_version": candidate_version(candidate_id)?,
            "idempotency_key": "safe-rollback-1"
        }),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let payload = response_json(response).await?;
    assert_eq!(payload["error"]["code"], "candidate_review_audit_failed");

    let conn = db::open_db()?;
    let (status, memory_count, ledger_count): (String, i64, i64) = conn.query_row(
        "SELECT c.review_status,
                (SELECT COUNT(*) FROM memories WHERE source_candidate_id = c.id),
                (SELECT COUNT(*) FROM api_mutation_requests)
         FROM memory_candidates c WHERE c.id = ?1",
        params![candidate_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(status, "pending_review");
    assert_eq!(memory_count, 0);
    assert_eq!(ledger_count, 0);
    Ok(())
}

#[tokio::test]
async fn staged_candidate_contract_stays_disabled_and_routes_require_auth() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-review-contract-disabled");
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::super::build_router(0).with_state(DbState);
    let capabilities = app
        .clone()
        .oneshot(authorized_json_request(
            Method::GET,
            "/api/v1/capabilities",
            &token,
            "",
        ))
        .await?;
    assert_eq!(capabilities.status(), StatusCode::OK);
    let capabilities = response_json(capabilities).await?;
    assert_eq!(capabilities["features"]["candidate_detail"], false);
    assert_eq!(capabilities["features"]["candidate_evidence"], false);
    assert_eq!(capabilities["features"]["candidate_review_safe"], false);
    for key in [
        "candidate_detail",
        "candidate_evidence",
        "candidate_review_safe_approve",
        "candidate_review_safe_reject",
        "candidate_review_safe_edit",
    ] {
        assert!(capabilities["endpoints"].get(key).is_none());
    }

    for uri in [
        "/api/v1/candidates/1",
        "/api/v1/candidates/1/review/approve",
        "/api/v1/candidates/1/review/reject",
        "/api/v1/candidates/1/review/edit",
    ] {
        let method = if uri.ends_with("/1") {
            Method::GET
        } else {
            Method::POST
        };
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

#[tokio::test]
async fn safe_review_validates_key_reason_and_candidate_before_mutation() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-review-validation");
    let (candidate_id, _) = insert_safe_review_candidate(
        "safe-review-validation",
        "session_stop",
        "raw-only evidence",
        "Candidate blocked by unsafe evidence.",
    )?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let version = candidate_version(candidate_id)?;

    let invalid_key = send_safe_review(
        candidate_id,
        "approve",
        &token,
        json!({
            "reason": "reviewed",
            "expected_version": version,
            "idempotency_key": "contains spaces"
        }),
    )
    .await?;
    assert_eq!(invalid_key.status(), StatusCode::BAD_REQUEST);
    let payload = response_json(invalid_key).await?;
    assert_eq!(payload["error"]["code"], "idempotency_key_invalid");
    assert_eq!(payload["error"]["operation_id"], Value::Null);

    let empty_reason = send_safe_review(
        candidate_id,
        "approve",
        &token,
        json!({
            "reason": "   ",
            "expected_version": version,
            "idempotency_key": "empty-reason-1"
        }),
    )
    .await?;
    assert_eq!(empty_reason.status(), StatusCode::BAD_REQUEST);
    let payload = response_json(empty_reason).await?;
    assert_eq!(payload["error"]["code"], "reason_invalid");
    assert!(payload["error"]["operation_id"].as_str().is_some());

    let blocked = send_safe_review(
        candidate_id,
        "approve",
        &token,
        json!({
            "reason": "reviewed",
            "expected_version": version,
            "idempotency_key": "blocked-evidence-1"
        }),
    )
    .await?;
    assert_eq!(blocked.status(), StatusCode::CONFLICT);
    let payload = response_json(blocked).await?;
    assert_eq!(payload["error"]["code"], "evidence_blocked");

    let missing = send_safe_review(
        9_999_999,
        "reject",
        &token,
        json!({
            "reason": "missing candidate",
            "expected_version": 0,
            "idempotency_key": "missing-candidate-1"
        }),
    )
    .await?;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let payload = response_json(missing).await?;
    assert_eq!(payload["error"]["code"], "candidate_not_found");

    let conn = db::open_db()?;
    let audit_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE event_type = 'candidate_review'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(audit_count, 0);
    Ok(())
}

#[tokio::test]
async fn safe_approve_requires_matching_quarantine_acknowledgement() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-review-quarantine");
    let (candidate_id, _) = insert_safe_review_candidate(
        "safe-review-quarantine",
        "file_edit",
        "safe evidence",
        "Quoted text was reviewed for poisoning.",
    )?;
    let conn = db::open_db()?;
    conn.execute(
        "UPDATE memory_candidates
         SET review_status = 'quarantined',
             quarantine_pattern_id = 'override_previous_instructions',
             quarantine_pattern_version = ?1
         WHERE id = ?2",
        params![
            crate::memory::poisoning::INSTRUCTION_PATTERN_SET_VERSION,
            candidate_id
        ],
    )?;
    drop(conn);
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let version = candidate_version(candidate_id)?;

    let missing_ack = send_safe_review(
        candidate_id,
        "approve",
        &token,
        json!({
            "reason": "reviewed quarantine",
            "expected_version": version,
            "idempotency_key": "quarantine-missing-ack"
        }),
    )
    .await?;
    assert_eq!(missing_ack.status(), StatusCode::CONFLICT);
    let payload = response_json(missing_ack).await?;
    assert_eq!(payload["error"]["code"], "candidate_review_rejected");

    let approved = send_safe_review(
        candidate_id,
        "approve",
        &token,
        json!({
            "reason": "reviewed quarantine",
            "expected_version": version,
            "idempotency_key": "quarantine-correct-ack",
            "acknowledge_pattern": "  override_previous_instructions  "
        }),
    )
    .await?;
    assert_eq!(approved.status(), StatusCode::OK);
    let payload = response_json(approved).await?;
    assert_eq!(payload["after_status"], "approved");

    let replay = send_safe_review(
        candidate_id,
        "approve",
        &token,
        json!({
            "reason": "reviewed quarantine",
            "expected_version": version,
            "idempotency_key": "quarantine-correct-ack",
            "acknowledge_pattern": "override_previous_instructions"
        }),
    )
    .await?;
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(response_json(replay).await?["replayed"], true);
    Ok(())
}

#[tokio::test]
async fn ledger_failure_rolls_back_candidate_memory_and_audit() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-review-ledger-rollback");
    let (candidate_id, _) = insert_safe_review_candidate(
        "safe-review-ledger-rollback",
        "file_edit",
        "safe evidence",
        "Candidate whose ledger insert fails.",
    )?;
    let conn = db::open_db()?;
    conn.execute_batch(
        "CREATE TRIGGER reject_candidate_review_ledger
         BEFORE INSERT ON api_mutation_requests
         BEGIN SELECT RAISE(ABORT, 'forced ledger failure'); END;",
    )?;
    drop(conn);
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let response = send_safe_review(
        candidate_id,
        "approve",
        &token,
        json!({
            "reason": "should roll back",
            "expected_version": candidate_version(candidate_id)?,
            "idempotency_key": "safe-ledger-rollback-1"
        }),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let payload = response_json(response).await?;
    assert_eq!(payload["error"]["code"], "candidate_review_ledger_failed");

    let conn = db::open_db()?;
    let (status, memory_count, audit_count): (String, i64, i64) = conn.query_row(
        "SELECT c.review_status,
                (SELECT COUNT(*) FROM memories WHERE source_candidate_id = c.id),
                (SELECT COUNT(*) FROM events WHERE event_type = 'candidate_review')
         FROM memory_candidates c WHERE c.id = ?1",
        params![candidate_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(status, "pending_review");
    assert_eq!(memory_count, 0);
    assert_eq!(audit_count, 0);
    Ok(())
}

#[test]
fn concurrent_same_key_rejects_apply_once_and_replay_once() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-review-concurrent-replay");
    let (candidate_id, _) = insert_safe_review_candidate(
        "safe-review-concurrent-replay",
        "file_edit",
        "safe evidence",
        "Candidate reviewed concurrently.",
    )?;
    let body = json!({
        "reason": "same concurrent decision",
        "expected_version": candidate_version(candidate_id)?,
        "idempotency_key": "concurrent-safe-review-1"
    });
    let barrier = Arc::new(Barrier::new(2));
    let connections = vec![db::open_db()?, db::open_db()?];
    let mut workers = Vec::new();
    for mut conn in connections {
        let barrier = Arc::clone(&barrier);
        let body = serde_json::to_vec(&body)?;
        workers.push(std::thread::spawn(move || -> anyhow::Result<Value> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            barrier.wait();
            let response = execute_safe_review_for_test(&mut conn, candidate_id, "reject", &body);
            runtime.block_on(async {
                anyhow::ensure!(response.status() == StatusCode::OK, "request failed");
                response_json(response).await
            })
        }));
    }
    let mut payloads = workers
        .into_iter()
        .map(|worker| {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("concurrent review worker panicked"))?
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    payloads.sort_by_key(|payload| payload["replayed"].as_bool());
    assert_eq!(payloads[0]["replayed"], false);
    assert_eq!(payloads[1]["replayed"], true);
    assert_eq!(payloads[0]["operation_id"], payloads[1]["operation_id"]);
    assert_eq!(payloads[0]["audit_id"], payloads[1]["audit_id"]);
    assert_eq!(payloads[0]["after_status"], "discarded");
    assert_eq!(payloads[1]["after_status"], "discarded");
    assert_eq!(payloads[0]["memory_id"], Value::Null);
    assert_eq!(payloads[1]["memory_id"], Value::Null);

    let conn = db::open_db()?;
    let (memory_count, audit_count, ledger_count): (i64, i64, i64) = conn.query_row(
        "SELECT
             (SELECT COUNT(*) FROM memories WHERE source_candidate_id = ?1),
             (SELECT COUNT(*) FROM events WHERE event_type = 'candidate_review'),
             (SELECT COUNT(*) FROM api_mutation_requests)",
        params![candidate_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(memory_count, 0);
    assert_eq!(audit_count, 1);
    assert_eq!(ledger_count, 1);
    Ok(())
}

#[tokio::test]
async fn safe_review_normalizes_business_body_and_preserves_audit_reason() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-review-normalized-body");
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let (reject_id, _) = insert_safe_review_candidate(
        "safe-review-normalized-reason",
        "file_edit",
        "raw evidence sentinel",
        "Candidate text must not enter audit.",
    )?;
    let reject_version = candidate_version(reject_id)?;
    let first = send_safe_review(
        reject_id,
        "reject",
        &token,
        json!({
            "reason": "  token=keep-exact  ",
            "expected_version": reject_version,
            "idempotency_key": "  normalized-reject-key  "
        }),
    )
    .await?;
    assert_eq!(first.status(), StatusCode::OK);
    let first = response_json(first).await?;
    let replay = send_safe_review(
        reject_id,
        "reject",
        &token,
        json!({
            "reason": "token=keep-exact",
            "expected_version": reject_version,
            "idempotency_key": "normalized-reject-key"
        }),
    )
    .await?;
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(response_json(replay).await?["replayed"], true);

    let conn = db::open_db()?;
    let audit_detail: String = conn.query_row(
        "SELECT detail FROM events WHERE id = ?1",
        params![first["audit_id"].as_i64().expect("audit id")],
        |row| row.get(0),
    )?;
    let audit: Value = serde_json::from_str(&audit_detail)?;
    assert_eq!(audit["reason"], "token=keep-exact");
    assert!(!audit_detail.contains("normalized-reject-key"));
    assert!(!audit_detail.contains("Candidate text must not enter audit."));
    drop(conn);

    let (edit_id, _) = insert_safe_review_candidate(
        "safe-review-normalized-edit",
        "file_edit",
        "safe evidence",
        "Candidate to normalize before editing.",
    )?;
    let edit_version = candidate_version(edit_id)?;
    let first = send_safe_review(
        edit_id,
        "edit",
        &token,
        json!({
            "reason": "  normalized edit  ",
            "expected_version": edit_version,
            "idempotency_key": "normalized-edit-key",
            "scope": " PROJECT ",
            "memory_type": " DECISION ",
            "topic_key": " Foo Bar ",
            "text": "  Use normalized review input.  "
        }),
    )
    .await?;
    assert_eq!(first.status(), StatusCode::OK);
    let replay = send_safe_review(
        edit_id,
        "edit",
        &token,
        json!({
            "reason": "normalized edit",
            "expected_version": edit_version,
            "idempotency_key": "normalized-edit-key",
            "scope": "project",
            "memory_type": "decision",
            "topic_key": "foo-bar",
            "text": "Use normalized review input."
        }),
    )
    .await?;
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(response_json(replay).await?["replayed"], true);
    Ok(())
}
