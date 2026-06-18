use axum::{
    body::to_bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use rusqlite::params;
use serde_json::Value;

use crate::db::test_support::ScopedTestDataDir;
use crate::{db, memory};

use super::super::handlers::{
    handle_graph, handle_list_candidates, handle_list_memories, handle_memory_detail,
};
use super::super::types::{CandidateParams, GraphParams, ListParams};
use super::super::DbState;

#[tokio::test]
async fn list_memories_project_filter_uses_repo_ownership_fields() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-list-owner-aware-project");
    let conn = db::open_db()?;
    let routed_id = memory::insert_memory(
        &conn,
        Some("session-routed"),
        "source-proj",
        None,
        "routed",
        "routed repo memory",
        "decision",
        None,
    )?;
    conn.execute(
        "UPDATE memories
         SET owner_scope = 'repo', owner_key = 'proj-a', target_project = 'proj-a'
         WHERE id = ?1",
        params![routed_id],
    )?;
    let legacy_id = memory::insert_memory(
        &conn,
        Some("session-legacy"),
        "proj-a",
        None,
        "legacy",
        "legacy project memory",
        "decision",
        None,
    )?;
    let other_id = memory::insert_memory(
        &conn,
        Some("session-other-owner"),
        "source-proj",
        None,
        "other",
        "other repo memory",
        "decision",
        None,
    )?;
    conn.execute(
        "UPDATE memories
         SET owner_scope = 'repo', owner_key = 'proj-b', target_project = 'proj-b'
         WHERE id = ?1",
        params![other_id],
    )?;
    drop(conn);

    let response = handle_list_memories(
        State(DbState),
        Query(ListParams {
            project: Some("proj-a".to_string()),
            memory_type: None,
            scope: None,
            status: None,
            branch: None,
            q: None,
            limit: Some(10),
            offset: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    let ids: Vec<i64> = payload["data"]
        .as_array()
        .expect("data should be array")
        .iter()
        .map(|item| item["id"].as_i64().expect("id should be i64"))
        .collect();
    assert!(ids.contains(&routed_id));
    assert!(ids.contains(&legacy_id));
    assert!(!ids.contains(&other_id));
    assert_eq!(payload["meta"]["total"], 2);
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
async fn memory_detail_marks_memory_accessed() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-detail-access");
    let conn = db::open_db()?;
    let memory_id = memory::insert_memory(
        &conn,
        Some("session-detail-access"),
        "proj-a",
        None,
        "detail access",
        "detail access memory",
        "decision",
        None,
    )?;
    drop(conn);

    let response = handle_memory_detail(State(DbState), Path(memory_id))
        .await
        .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let conn = db::open_db()?;
    let access_count: i64 = conn.query_row(
        "SELECT access_count FROM memories WHERE id = ?1",
        params![memory_id],
        |row| row.get(0),
    )?;
    assert_eq!(access_count, 1);
    Ok(())
}

#[tokio::test]
async fn graph_nodes_are_ranked_from_current_memory_links() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-graph-current-nodes");
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO entities (id, canonical_name, entity_type, mention_count, created_at_epoch)
         VALUES
          (1, 'stale-only', 'topic', 999, 1),
          (2, 'current', 'topic', 1, 1)",
        [],
    )?;
    let stale_id = memory::insert_memory(
        &conn,
        Some("session-stale-node"),
        "proj-a",
        None,
        "stale node",
        "stale node memory",
        "decision",
        None,
    )?;
    let current_id = memory::insert_memory(
        &conn,
        Some("session-current-node"),
        "proj-a",
        None,
        "current node",
        "current node memory",
        "decision",
        None,
    )?;
    conn.execute(
        "UPDATE memories SET status = 'stale' WHERE id = ?1",
        params![stale_id],
    )?;
    conn.execute(
        "INSERT INTO memory_entities (memory_id, entity_id) VALUES (?1, 1), (?2, 2)",
        params![stale_id, current_id],
    )?;
    drop(conn);

    let response = handle_graph(State(DbState), Query(GraphParams { limit: Some(1) }))
        .await
        .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["nodes"][0]["id"], 2);
    assert_eq!(payload["nodes"][0]["mems"], serde_json::json!([current_id]));
    Ok(())
}
