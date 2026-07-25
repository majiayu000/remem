use axum::http::StatusCode;
use rusqlite::params;
use serde::Serialize;
use serde_json::json;

use crate::api::mutation::{
    mutation_request_hash, validate_idempotency_key, CredentialFreeMutationBody,
};
use crate::db;
use crate::db::test_support::ScopedTestDataDir;

use super::insert_safe_review_candidate;
use super::{candidate_version, response_json, send_safe_review};

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

#[derive(Serialize)]
struct RejectHashFixture<'a> {
    reason: &'a str,
    expected_version: i64,
}

impl CredentialFreeMutationBody for RejectHashFixture<'_> {}
