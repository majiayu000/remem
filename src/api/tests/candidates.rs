use axum::{
    body::to_bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::Value;

use crate::db;
use crate::db::test_support::ScopedTestDataDir;

use super::super::handlers::{handle_candidate_detail, handle_list_candidates};
use super::super::types::CandidateParams;
use super::super::DbState;
use super::candidate_safe_review::insert_safe_review_candidate;

async fn candidate_detail_payload(id: i64) -> anyhow::Result<(StatusCode, Value)> {
    let response = handle_candidate_detail(State(DbState), Path(id))
        .await
        .into_response();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    Ok((status, serde_json::from_slice(&body)?))
}

#[tokio::test]
async fn list_candidates_defaults_to_pending_review() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidates-pending-review");
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO memory_candidates
          (scope, memory_type, topic_key, text, evidence_event_ids, confidence,
           risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES
          ('project', 'decision', 'candidate-topic', 'needs review', '[]', 0.7,
           'low', 'pending_review', 1, 1)",
        [],
    )?;
    drop(conn);

    let response = handle_list_candidates(
        State(DbState),
        Query(CandidateParams {
            project: None,
            status: None,
            memory_type: None,
            block_reason: None,
            topic_key: None,
            contains: None,
            min_confidence: None,
            older_than_days: None,
            limit: Some(10),
            offset: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["meta"]["total"], 1);
    assert_eq!(payload["data"][0]["review_status"], "pending_review");
    Ok(())
}

#[tokio::test]
async fn list_candidates_filters_by_project_when_requested() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidates-project-filter");
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO workspaces (id, root_path, created_at_epoch, updated_at_epoch)
         VALUES (1, 'workspace-a', 1, 1)",
        [],
    )?;
    conn.execute(
        "INSERT INTO projects (id, workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES
          (10, 1, 'proj-a', 'proj-a', 1, 1),
          (11, 1, 'proj-b', 'proj-b', 1, 1)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_candidates
          (project_id, scope, memory_type, topic_key, text, evidence_event_ids, confidence,
           risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES
          (10, 'project', 'decision', 'candidate-a', 'project candidate', '[]', 0.7,
           'low', 'pending_review', 3, 3),
          (11, 'project', 'decision', 'candidate-b', 'other candidate', '[]', 0.7,
           'low', 'pending_review', 2, 2)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_candidates
          (project_id, scope, memory_type, topic_key, text, evidence_event_ids, confidence,
           risk_class, review_status, source_project, target_project, owner_scope, owner_key,
           created_at_epoch, updated_at_epoch)
         VALUES
          (11, 'project', 'decision', 'candidate-routed', 'routed candidate', '[]', 0.7,
           'low', 'pending_review', 'proj-b', 'proj-a', 'repo', 'proj-a', 4, 4)",
        [],
    )?;
    drop(conn);

    let response = handle_list_candidates(
        State(DbState),
        Query(CandidateParams {
            project: Some("proj-a".to_string()),
            status: None,
            memory_type: None,
            block_reason: None,
            topic_key: None,
            contains: None,
            min_confidence: None,
            older_than_days: None,
            limit: Some(10),
            offset: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    let texts: Vec<&str> = payload["data"]
        .as_array()
        .expect("data should be array")
        .iter()
        .map(|item| item["text"].as_str().expect("text should be string"))
        .collect();
    assert!(texts.contains(&"project candidate"));
    assert!(texts.contains(&"routed candidate"));
    assert!(!texts.contains(&"other candidate"));
    assert_eq!(payload["data"][0]["project"], "proj-a");
    assert_eq!(payload["meta"]["total"], 2);
    Ok(())
}

#[tokio::test]
async fn list_candidates_contains_filter_treats_like_wildcards_literally() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidates-like-escape");
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO memory_candidates
          (scope, memory_type, topic_key, text, evidence_event_ids, confidence,
           risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES
          ('project', 'decision', 'percent', 'contains 100% literal', '[]', 0.7,
           'low', 'pending_review', 1, 1),
          ('project', 'decision', 'ordinary', 'contains ordinary text', '[]', 0.7,
           'low', 'pending_review', 2, 2)",
        [],
    )?;
    drop(conn);

    let response = handle_list_candidates(
        State(DbState),
        Query(CandidateParams {
            project: None,
            status: None,
            memory_type: None,
            block_reason: None,
            topic_key: None,
            contains: Some("%".to_string()),
            min_confidence: None,
            older_than_days: None,
            limit: Some(10),
            offset: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["meta"]["total"], 1);
    assert_eq!(payload["data"][0]["topic_key"], Value::Null);
    assert_eq!(payload["data"][0]["text"], "contains 100% literal");
    Ok(())
}

#[tokio::test]
async fn list_candidates_rejects_negative_older_than() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidates-negative-age");
    let response = handle_list_candidates(
        State(DbState),
        Query(CandidateParams {
            project: None,
            status: None,
            memory_type: None,
            block_reason: None,
            topic_key: None,
            contains: None,
            min_confidence: None,
            older_than_days: Some(-1),
            limit: Some(10),
            offset: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["error"]["code"], "candidate_filter_invalid");
    Ok(())
}

#[tokio::test]
async fn list_candidates_rejects_oversized_older_than() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidates-oversized-age");
    let response = handle_list_candidates(
        State(DbState),
        Query(CandidateParams {
            project: None,
            status: None,
            memory_type: None,
            block_reason: None,
            topic_key: None,
            contains: None,
            min_confidence: None,
            older_than_days: Some(i64::MAX),
            limit: Some(10),
            offset: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["error"]["code"], "candidate_filter_invalid");
    assert!(payload["error"]["message"]
        .as_str()
        .expect("error message should be string")
        .contains("too large"));
    Ok(())
}

#[tokio::test]
async fn list_candidates_rejects_out_of_range_min_confidence() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidates-invalid-confidence");
    for min_confidence in [-0.1, 1.1, f64::NAN] {
        let response = handle_list_candidates(
            State(DbState),
            Query(CandidateParams {
                project: None,
                status: None,
                memory_type: None,
                block_reason: None,
                topic_key: None,
                contains: None,
                min_confidence: Some(min_confidence),
                older_than_days: None,
                limit: Some(10),
                offset: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let payload: Value = serde_json::from_slice(&body)?;
        assert_eq!(payload["error"]["code"], "candidate_filter_invalid");
    }
    Ok(())
}

#[tokio::test]
async fn candidate_detail_projects_safe_metadata_without_raw_evidence() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidate-detail-safe");
    let raw_sentinel = "RAW_EVIDENCE_SENTINEL token=super-secret-value";
    let (candidate_id, event_id) = insert_safe_review_candidate(
        "candidate-detail-safe",
        "file_edit",
        raw_sentinel,
        "Use a transaction for review writes.",
    )?;

    let (status, payload) = candidate_detail_payload(candidate_id).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["data"]["id"], candidate_id);
    assert_eq!(payload["evidence"][0]["source_id"], event_id);
    assert_eq!(payload["evidence"][0]["event_type"], "file_edit");
    assert_eq!(payload["evidence"][0]["summary"], "File edit evidence");
    assert_eq!(payload["evidence"][0]["preview"], "");
    assert_eq!(payload["evidence"][0]["redacted"], true);
    assert_eq!(payload["decision"]["can_review"], true);
    let serialized = serde_json::to_string(&payload)?;
    assert!(!serialized.contains(raw_sentinel));
    assert!(!serialized.contains("super-secret-value"));
    Ok(())
}

#[tokio::test]
async fn candidate_detail_fails_closed_for_unsafe_event_type() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidate-detail-unsafe");
    let (candidate_id, _) = insert_safe_review_candidate(
        "candidate-detail-unsafe",
        "session_stop",
        "raw-only transcript",
        "Candidate derived from a stopped session.",
    )?;

    let (status, payload) = candidate_detail_payload(candidate_id).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["decision"]["can_review"], false);
    assert_eq!(
        payload["decision"]["blocked_reasons"],
        serde_json::json!(["evidence_safe_projection_unavailable"])
    );
    assert_eq!(payload["evidence"][0]["summary"], "");
    assert_eq!(payload["evidence"][0]["preview"], "");
    assert_eq!(
        payload["evidence"][0]["provenance_status"],
        "unsafe_projection"
    );
    Ok(())
}

#[tokio::test]
async fn candidate_detail_fails_closed_for_missing_and_cross_project_evidence() -> anyhow::Result<()>
{
    let _test_dir = ScopedTestDataDir::new("api-candidate-detail-provenance");
    let (candidate_id, _) = insert_safe_review_candidate(
        "candidate-detail-home",
        "file_edit",
        "home evidence",
        "Candidate with invalid provenance.",
    )?;
    let (_, cross_event_id) = insert_safe_review_candidate(
        "candidate-detail-other",
        "file_edit",
        "other evidence",
        "Other candidate.",
    )?;
    let conn = db::open_db()?;
    conn.execute(
        "UPDATE memory_candidates SET evidence_event_ids = ?1 WHERE id = ?2",
        rusqlite::params![
            serde_json::to_string(&vec![cross_event_id, 9_999_999_i64])?,
            candidate_id
        ],
    )?;
    drop(conn);

    let (status, payload) = candidate_detail_payload(candidate_id).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["decision"]["can_review"], false);
    let reasons = payload["decision"]["blocked_reasons"]
        .as_array()
        .expect("blocked reasons should be an array");
    assert!(reasons.contains(&Value::String("evidence_cross_project".to_string())));
    assert!(reasons.contains(&Value::String("evidence_missing".to_string())));
    assert_eq!(payload["evidence"][0]["event_type"], Value::Null);
    assert_eq!(payload["evidence"][1]["event_type"], Value::Null);
    Ok(())
}

#[tokio::test]
async fn candidate_detail_rejects_malformed_nonpositive_and_duplicate_evidence_ids(
) -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidate-detail-invalid-ids");
    let (candidate_id, event_id) = insert_safe_review_candidate(
        "candidate-detail-invalid-ids",
        "file_edit",
        "safe evidence",
        "Candidate with malformed references.",
    )?;
    let cases = [
        ("not-json", "evidence_ids_invalid"),
        ("[0]", "evidence_id_invalid"),
        (&format!("[{event_id},{event_id}]"), "evidence_id_duplicate"),
    ];
    for (evidence_json, expected_reason) in cases {
        let conn = db::open_db()?;
        conn.execute(
            "UPDATE memory_candidates SET evidence_event_ids = ?1 WHERE id = ?2",
            rusqlite::params![evidence_json, candidate_id],
        )?;
        drop(conn);
        let (_, payload) = candidate_detail_payload(candidate_id).await?;
        assert_eq!(payload["decision"]["can_review"], false);
        assert_eq!(
            payload["decision"]["blocked_reasons"],
            serde_json::json!([expected_reason])
        );
        assert_eq!(payload["evidence"], serde_json::json!([]));
    }
    Ok(())
}

#[tokio::test]
async fn candidate_detail_applies_active_candidate_suppression() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidate-detail-suppressed");
    let (candidate_id, _) = insert_safe_review_candidate(
        "candidate-detail-suppressed",
        "file_edit",
        "safe evidence",
        "Candidate suppressed by policy.",
    )?;
    let conn = db::open_db()?;
    let topic_key: String = conn.query_row(
        "SELECT topic_key FROM memory_candidates WHERE id = ?1",
        rusqlite::params![candidate_id],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO memory_suppressions
         (target_kind, target_value, reason, actor, status, created_at_epoch, updated_at_epoch)
         VALUES ('topic_key', ?1, 'policy decision', 'test', 'active', 1, 1)",
        rusqlite::params![topic_key],
    )?;
    drop(conn);

    let (_, payload) = candidate_detail_payload(candidate_id).await?;
    assert_eq!(payload["decision"]["can_review"], false);
    assert!(payload["decision"]["blocked_reasons"]
        .as_array()
        .expect("blocked reasons should be an array")
        .contains(&Value::String("candidate_policy_suppressed".to_string())));
    Ok(())
}

#[tokio::test]
async fn candidate_detail_requires_at_least_one_evidence_reference() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidate-detail-empty-evidence");
    let (candidate_id, _) = insert_safe_review_candidate(
        "candidate-detail-empty-evidence",
        "file_edit",
        "safe evidence",
        "Candidate whose evidence list is empty.",
    )?;
    let conn = db::open_db()?;
    conn.execute(
        "UPDATE memory_candidates SET evidence_event_ids = '[]' WHERE id = ?1",
        rusqlite::params![candidate_id],
    )?;
    drop(conn);

    let (_, payload) = candidate_detail_payload(candidate_id).await?;
    assert_eq!(payload["decision"]["can_review"], false);
    assert_eq!(
        payload["decision"]["blocked_reasons"],
        serde_json::json!(["evidence_required"])
    );
    assert_eq!(payload["evidence"], serde_json::json!([]));
    Ok(())
}

#[tokio::test]
async fn candidate_detail_ignores_user_context_candidate_id_suppressions() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidate-detail-id-domain");
    let (candidate_id, _) = insert_safe_review_candidate(
        "candidate-detail-id-domain",
        "file_edit",
        "safe evidence",
        "Memory candidate in a distinct identifier domain.",
    )?;
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO memory_suppressions
         (target_kind, target_id, reason, actor, status, created_at_epoch, updated_at_epoch)
         VALUES ('user_candidate', ?1, 'different id domain', 'test', 'active', 1, 1)",
        rusqlite::params![candidate_id],
    )?;
    drop(conn);

    let (_, payload) = candidate_detail_payload(candidate_id).await?;
    assert_eq!(payload["decision"]["can_review"], true);
    Ok(())
}

#[tokio::test]
async fn candidate_detail_applies_pattern_suppression_before_redaction() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidate-detail-pattern-order");
    let secret = "candidate-policy-secret-value";
    let (candidate_id, _) = insert_safe_review_candidate(
        "candidate-detail-pattern-order",
        "file_edit",
        "safe evidence",
        &format!("token={secret}"),
    )?;
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO memory_suppressions
         (target_kind, target_value, reason, actor, status, created_at_epoch, updated_at_epoch)
         VALUES ('pattern', ?1, 'secret policy', 'test', 'active', 1, 1)",
        rusqlite::params![secret],
    )?;
    drop(conn);

    let (_, payload) = candidate_detail_payload(candidate_id).await?;
    assert_eq!(payload["decision"]["can_review"], false);
    assert!(payload["decision"]["blocked_reasons"]
        .as_array()
        .expect("blocked reasons should be an array")
        .contains(&Value::String("candidate_policy_suppressed".to_string())));
    assert!(!serde_json::to_string(&payload)?.contains(secret));
    Ok(())
}
