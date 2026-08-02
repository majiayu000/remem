use axum::http::StatusCode;
use rusqlite::params;
use serde::Serialize;
use serde_json::json;

use crate::api::mutation::{
    mutation_request_hash, validate_idempotency_key, CredentialFreeMutationBody,
};
use crate::db;
use crate::db::test_support::ScopedTestDataDir;

use super::{candidate_version, response_json, send_safe_review};
use super::{insert_safe_dream_review_candidate, insert_safe_review_candidate};

mod dream_policy;

#[tokio::test]
async fn safe_review_distinguishes_nonreviewable_and_unknown_replay_schema() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-review-stable-errors");
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let (candidate_id, _) = insert_safe_review_candidate(
        "safe-review-stable-errors",
        "file_edit",
        "safe evidence",
        "Candidate with stable error responses.",
    )?;
    let conn = db::open_db()?;
    conn.execute(
        "UPDATE memory_candidates SET review_status = 'discarded' WHERE id = ?1",
        params![candidate_id],
    )?;
    drop(conn);
    let version = candidate_version(candidate_id)?;
    let response = send_safe_review(
        candidate_id,
        "reject",
        &token,
        json!({
            "reason": "cannot review again",
            "expected_version": version,
            "idempotency_key": "nonreviewable-fresh-key"
        }),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await?["error"]["code"],
        "candidate_not_reviewable"
    );

    let (schema_id, _) = insert_safe_review_candidate(
        "safe-review-unknown-schema",
        "file_edit",
        "safe evidence",
        "Candidate whose replay schema is unknown.",
    )?;
    let schema_version = candidate_version(schema_id)?;
    let identity = validate_idempotency_key("unknown-schema-key")?;
    let request_hash = mutation_request_hash(
        "candidate",
        schema_id,
        "reject",
        &RejectHashFixture {
            reason: "schema replay",
            expected_version: schema_version,
        },
    )?;
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO api_mutation_requests(
             idempotency_key_hash, request_hash, operation_id, resource_kind,
             resource_id, action, response_schema_version, response_json,
             audit_id, created_at_epoch)
         VALUES (?1, ?2, ?3, 'candidate', ?4, 'reject', 99, '{}', 1, 1)",
        params![
            identity.idempotency_key_hash,
            request_hash,
            identity.operation_id,
            schema_id
        ],
    )?;
    drop(conn);
    let response = send_safe_review(
        schema_id,
        "reject",
        &token,
        json!({
            "reason": "schema replay",
            "expected_version": schema_version,
            "idempotency_key": "unknown-schema-key"
        }),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await?["error"]["code"],
        "idempotency_schema_unsupported"
    );
    Ok(())
}

#[tokio::test]
async fn safe_dream_semantic_supersede_invalidates_old_token_without_side_effects(
) -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-safe-dream-semantic-supersede");
    let (candidate_id, member_ids) =
        insert_safe_dream_review_candidate("safe-dream-semantic-supersede")?;
    let conn = db::open_db()?;
    let old_token =
        crate::memory_candidate::review::load_dream_quarantine_provenance(&conn, candidate_id)?
            .and_then(|provenance| provenance.review_token)
            .ok_or_else(|| anyhow::anyhow!("old Dream review token missing"))?;
    conn.execute(
        "UPDATE memory_candidates
         SET review_status = 'discarded',
             review_actor = 'system:dream',
             reviewed_at_epoch = updated_at_epoch,
             review_action_source = 'dream_semantic_superseded',
             review_reason = 'superseded_by_newer_dream_semantic_decision'
         WHERE id = ?1 AND review_status = 'quarantined'",
        params![candidate_id],
    )?;
    drop(conn);
    crate::api::ensure_api_token()?;
    let api_token = crate::api::load_api_token()?;

    let response = send_safe_review(
        candidate_id,
        "approve",
        &api_token,
        json!({
            "reason": "old semantic review must remain invalid",
            "expected_version": candidate_version(candidate_id)?,
            "idempotency_key": "dream-semantic-superseded-old-token",
            "acknowledge_pattern": "override_previous_instructions",
            "acknowledge_dream_review_token": old_token
        }),
    )
    .await?;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await?["error"]["code"],
        "candidate_not_reviewable"
    );
    let conn = db::open_db()?;
    let (status, promoted, audits, ledger): (String, i64, i64, i64) = conn.query_row(
        "SELECT c.review_status,
                (SELECT COUNT(*) FROM memories WHERE source_candidate_id = c.id),
                (SELECT COUNT(*) FROM events
                 WHERE event_type IN ('candidate_review', 'candidate_dream_review')),
                (SELECT COUNT(*) FROM api_mutation_requests)
         FROM memory_candidates c WHERE c.id = ?1",
        params![candidate_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(status, "discarded");
    assert_eq!((promoted, audits, ledger), (0, 0, 0));
    for member_id in member_ids {
        let source_status: String = conn.query_row(
            "SELECT status FROM memories WHERE id = ?1",
            params![member_id],
            |row| row.get(0),
        )?;
        assert_eq!(source_status, "active");
    }
    Ok(())
}

#[derive(Serialize)]
struct RejectHashFixture<'a> {
    reason: &'a str,
    expected_version: i64,
}

impl CredentialFreeMutationBody for RejectHashFixture<'_> {}
