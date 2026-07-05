use axum::{
    body::to_bytes,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::Value;

use crate::db;
use crate::db::test_support::ScopedTestDataDir;

use super::super::handlers::handle_list_candidates;
use super::super::types::CandidateParams;
use super::super::DbState;

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
