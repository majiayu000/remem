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
use super::super::types::{CandidateParams, GraphParams, ListParams, MemoryDetailParams};
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
            include_suppressed: None,
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
    let routed_item = payload["data"]
        .as_array()
        .expect("data should be array")
        .iter()
        .find(|item| item["id"] == routed_id)
        .expect("routed item should be returned");
    assert_eq!(routed_item["project"], "proj-a");
    assert_eq!(payload["meta"]["total"], 2);
    Ok(())
}

#[tokio::test]
async fn list_memories_scope_filter_treats_null_as_project() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-list-null-scope-project");
    let conn = db::open_db()?;
    let legacy_null_scope_id = memory::insert_memory(
        &conn,
        Some("session-null-scope"),
        "proj-a",
        None,
        "null scope",
        "legacy null scope memory",
        "decision",
        None,
    )?;
    conn.execute(
        "UPDATE memories SET scope = NULL WHERE id = ?1",
        params![legacy_null_scope_id],
    )?;
    let other_id = memory::insert_memory(
        &conn,
        Some("session-other-scope"),
        "proj-a",
        None,
        "global scope",
        "global scope memory",
        "decision",
        None,
    )?;
    conn.execute(
        "UPDATE memories SET scope = 'global' WHERE id = ?1",
        params![other_id],
    )?;
    drop(conn);

    let response = handle_list_memories(
        State(DbState),
        Query(ListParams {
            project: Some("proj-a".to_string()),
            memory_type: None,
            scope: Some("project".to_string()),
            status: None,
            branch: None,
            q: None,
            include_suppressed: None,
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
    assert!(ids.contains(&legacy_null_scope_id));
    assert!(!ids.contains(&other_id));
    assert_eq!(payload["meta"]["total"], 1);
    Ok(())
}

#[tokio::test]
async fn list_memories_active_status_excludes_superseded_state_key_rows() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-list-active-current-state-key");
    let conn = db::open_db()?;
    let old_id = memory::insert_memory(
        &conn,
        Some("session-old-state"),
        "proj-a",
        None,
        "old state",
        "old active state memory",
        "decision",
        None,
    )?;
    let current_id = memory::insert_memory(
        &conn,
        Some("session-current-state"),
        "proj-a",
        None,
        "current state",
        "current active state memory",
        "decision",
        None,
    )?;
    conn.execute(
        "INSERT INTO memory_state_keys
          (id, owner_scope, owner_key, memory_type, state_key, state_label,
           state_status, current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (10, 'repo', 'proj-a', 'decision', 'decision:test', 'test',
           'active', ?1, 1, 2)",
        params![current_id],
    )?;
    conn.execute(
        "UPDATE memories SET state_key_id = 10 WHERE id IN (?1, ?2)",
        params![old_id, current_id],
    )?;
    conn.execute(
        "INSERT INTO memory_edges
          (edge_type, from_memory_id, to_memory_id, state_key_id, reason, created_at_epoch)
         VALUES ('supersedes', ?1, ?2, 10, 'test replacement', 3)",
        params![old_id, current_id],
    )?;
    drop(conn);

    let response = handle_list_memories(
        State(DbState),
        Query(ListParams {
            project: Some("proj-a".to_string()),
            memory_type: None,
            scope: None,
            status: Some("active".to_string()),
            branch: None,
            q: None,
            include_suppressed: None,
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
    assert!(ids.contains(&current_id));
    assert!(!ids.contains(&old_id));
    assert_eq!(payload["meta"]["total"], 1);
    Ok(())
}

#[tokio::test]
async fn list_memories_active_status_keeps_unsuperseded_state_key_siblings() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-list-active-state-key-siblings");
    let conn = db::open_db()?;
    let first_id = memory::insert_memory(
        &conn,
        Some("session-first-state-sibling"),
        "proj-a",
        Some("decision-11111111"),
        "first state sibling",
        "Use FTS5 trigram tokenizer for CJK text search support.",
        "decision",
        None,
    )?;
    let second_id = memory::insert_memory(
        &conn,
        Some("session-second-state-sibling"),
        "proj-a",
        Some("decision-22222222"),
        "second state sibling",
        "Switch CJK search to FTS5 trigram tokenization.",
        "decision",
        None,
    )?;
    conn.execute(
        "INSERT INTO memory_state_keys
          (id, owner_scope, owner_key, memory_type, state_key, state_label,
           state_status, current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (10, 'repo', 'proj-a', 'decision', 'decision:semantic-slot', 'slot',
           'active', ?1, 1, 2)",
        params![second_id],
    )?;
    conn.execute(
        "UPDATE memories SET state_key_id = 10 WHERE id IN (?1, ?2)",
        params![first_id, second_id],
    )?;
    drop(conn);

    let response = handle_list_memories(
        State(DbState),
        Query(ListParams {
            project: Some("proj-a".to_string()),
            memory_type: None,
            scope: None,
            status: Some("active".to_string()),
            branch: None,
            q: None,
            include_suppressed: None,
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
    assert!(ids.contains(&first_id));
    assert!(ids.contains(&second_id));
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

    let response = handle_memory_detail(
        State(DbState),
        Path(memory_id),
        Query(MemoryDetailParams {
            include_suppressed: None,
        }),
    )
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
async fn list_memories_fails_when_source_anchor_label_fails() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-list-staleness-source-error");
    let conn = db::open_db()?;
    let memory_id = memory::insert_memory(
        &conn,
        Some("session-bad-staleness"),
        "proj-a",
        None,
        "bad staleness",
        "bad source-anchor fixture",
        "decision",
        None,
    )?;
    conn.execute(
        "UPDATE memories SET files = '[not-json' WHERE id = ?1",
        params![memory_id],
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
            include_suppressed: None,
            limit: Some(10),
            offset: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["error"]["code"], "staleness_source_anchor_failed");
    let conn = db::open_db()?;
    let access_count: i64 = conn.query_row(
        "SELECT access_count FROM memories WHERE id = ?1",
        params![memory_id],
        |row| row.get(0),
    )?;
    assert_eq!(access_count, 0);
    Ok(())
}

#[tokio::test]
async fn memory_detail_fails_when_source_anchor_label_fails() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-detail-staleness-source-error");
    let conn = db::open_db()?;
    let memory_id = memory::insert_memory(
        &conn,
        Some("session-detail-bad-staleness"),
        "proj-a",
        None,
        "bad detail staleness",
        "bad detail source-anchor fixture",
        "decision",
        None,
    )?;
    conn.execute(
        "UPDATE memories SET files = '[not-json' WHERE id = ?1",
        params![memory_id],
    )?;
    drop(conn);

    let response = handle_memory_detail(
        State(DbState),
        Path(memory_id),
        Query(MemoryDetailParams {
            include_suppressed: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["error"]["code"], "staleness_source_anchor_failed");
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

    let response = handle_graph(
        State(DbState),
        Query(GraphParams {
            project: None,
            include_suppressed: None,
            limit: Some(1),
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["nodes"][0]["id"], 2);
    assert_eq!(payload["nodes"][0]["mems"], serde_json::json!([current_id]));
    Ok(())
}

#[tokio::test]
async fn graph_empty_database_returns_empty_arrays() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-graph-empty");
    let conn = db::open_db()?;
    drop(conn);

    let response = handle_graph(
        State(DbState),
        Query(GraphParams {
            project: None,
            include_suppressed: None,
            limit: Some(10),
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["nodes"], serde_json::json!([]));
    assert_eq!(payload["edges"], serde_json::json!([]));
    Ok(())
}

#[tokio::test]
async fn graph_edges_use_stable_tie_breaker_after_weight() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-graph-edge-order");
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO entities (id, canonical_name, entity_type, mention_count, created_at_epoch)
         VALUES
          (1, 'alpha', 'topic', 1, 1),
          (2, 'beta', 'topic', 1, 1),
          (3, 'gamma', 'topic', 1, 1)",
        [],
    )?;
    let memory_id = memory::insert_memory(
        &conn,
        Some("session-edge-order"),
        "proj-a",
        None,
        "edge order graph row",
        "edge order graph memory",
        "decision",
        None,
    )?;
    conn.execute(
        "INSERT INTO memory_entities (memory_id, entity_id) VALUES
         (?1, 3),
         (?1, 1),
         (?1, 2)",
        params![memory_id],
    )?;
    drop(conn);

    let response = handle_graph(
        State(DbState),
        Query(GraphParams {
            project: None,
            include_suppressed: None,
            limit: Some(10),
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(
        payload["edges"],
        serde_json::json!([
            { "a": 1, "b": 2, "w": 1 },
            { "a": 1, "b": 3, "w": 1 },
            { "a": 2, "b": 3, "w": 1 }
        ])
    );
    Ok(())
}

#[tokio::test]
async fn graph_project_filter_scopes_nodes_and_memory_links() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-graph-project-scope");
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO entities (id, canonical_name, entity_type, mention_count, created_at_epoch)
         VALUES
          (1, 'shared', 'topic', 100, 1),
          (2, 'other-only', 'topic', 99, 1)",
        [],
    )?;
    let project_memory_id = memory::insert_memory(
        &conn,
        Some("session-project-graph"),
        "proj-a",
        None,
        "project graph row",
        "project graph memory",
        "decision",
        None,
    )?;
    let routed_memory_id = memory::insert_memory(
        &conn,
        Some("session-routed-graph"),
        "source-proj",
        None,
        "routed graph row",
        "routed graph memory",
        "decision",
        None,
    )?;
    conn.execute(
        "UPDATE memories
         SET owner_scope = 'repo', owner_key = 'proj-a', target_project = 'proj-a'
         WHERE id = ?1",
        params![routed_memory_id],
    )?;
    let other_memory_id = memory::insert_memory(
        &conn,
        Some("session-other-graph"),
        "proj-b",
        None,
        "other graph row",
        "other graph memory",
        "decision",
        None,
    )?;
    conn.execute(
        "INSERT INTO memory_entities (memory_id, entity_id) VALUES
         (?1, 1),
         (?2, 1),
         (?3, 1),
         (?3, 2)",
        params![project_memory_id, routed_memory_id, other_memory_id],
    )?;
    drop(conn);

    let response = handle_graph(
        State(DbState),
        Query(GraphParams {
            project: Some("proj-a".to_string()),
            include_suppressed: None,
            limit: Some(10),
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    let nodes = payload["nodes"].as_array().expect("nodes should be array");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["id"], 1);
    let mems: Vec<i64> = nodes[0]["mems"]
        .as_array()
        .expect("mems should be array")
        .iter()
        .map(|id| id.as_i64().expect("memory id should be i64"))
        .collect();
    assert!(mems.contains(&project_memory_id));
    assert!(mems.contains(&routed_memory_id));
    assert!(!mems.contains(&other_memory_id));
    Ok(())
}

#[tokio::test]
async fn graph_excludes_superseded_state_key_memory_links() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-graph-current-state-key");
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO entities (id, canonical_name, entity_type, mention_count, created_at_epoch)
         VALUES
          (1, 'old-only', 'topic', 100, 1),
          (2, 'current-only', 'topic', 1, 1)",
        [],
    )?;
    let old_id = memory::insert_memory(
        &conn,
        Some("session-old-graph-state"),
        "proj-a",
        None,
        "old graph state",
        "old graph state memory",
        "decision",
        None,
    )?;
    let current_id = memory::insert_memory(
        &conn,
        Some("session-current-graph-state"),
        "proj-a",
        None,
        "current graph state",
        "current graph state memory",
        "decision",
        None,
    )?;
    conn.execute(
        "INSERT INTO memory_state_keys
          (id, owner_scope, owner_key, memory_type, state_key, state_label,
           state_status, current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (10, 'repo', 'proj-a', 'decision', 'decision:graph', 'graph',
           'active', ?1, 1, 2)",
        params![current_id],
    )?;
    conn.execute(
        "UPDATE memories SET state_key_id = 10 WHERE id IN (?1, ?2)",
        params![old_id, current_id],
    )?;
    conn.execute(
        "INSERT INTO memory_edges
          (edge_type, from_memory_id, to_memory_id, state_key_id, reason, created_at_epoch)
         VALUES ('supersedes', ?1, ?2, 10, 'test replacement', 3)",
        params![old_id, current_id],
    )?;
    conn.execute(
        "INSERT INTO memory_entities (memory_id, entity_id) VALUES (?1, 1), (?1, 2), (?2, 2)",
        params![old_id, current_id],
    )?;
    drop(conn);

    let response = handle_graph(
        State(DbState),
        Query(GraphParams {
            project: Some("proj-a".to_string()),
            include_suppressed: None,
            limit: Some(10),
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    let nodes = payload["nodes"].as_array().expect("nodes should be array");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["id"], 2);
    assert_eq!(nodes[0]["mems"], serde_json::json!([current_id]));
    Ok(())
}

#[tokio::test]
async fn graph_keeps_unsuperseded_state_key_memory_links() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-graph-state-key-siblings");
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO entities (id, canonical_name, entity_type, mention_count, created_at_epoch)
         VALUES (1, 'shared', 'topic', 10, 1)",
        [],
    )?;
    let first_id = memory::insert_memory(
        &conn,
        Some("session-first-graph-sibling"),
        "proj-a",
        Some("decision-11111111"),
        "first graph sibling",
        "Use FTS5 trigram tokenizer for CJK text search support.",
        "decision",
        None,
    )?;
    let second_id = memory::insert_memory(
        &conn,
        Some("session-second-graph-sibling"),
        "proj-a",
        Some("decision-22222222"),
        "second graph sibling",
        "Switch CJK search to FTS5 trigram tokenization.",
        "decision",
        None,
    )?;
    conn.execute(
        "INSERT INTO memory_state_keys
          (id, owner_scope, owner_key, memory_type, state_key, state_label,
           state_status, current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (10, 'repo', 'proj-a', 'decision', 'decision:semantic-slot', 'slot',
           'active', ?1, 1, 2)",
        params![second_id],
    )?;
    conn.execute(
        "UPDATE memories SET state_key_id = 10 WHERE id IN (?1, ?2)",
        params![first_id, second_id],
    )?;
    conn.execute(
        "INSERT INTO memory_entities (memory_id, entity_id) VALUES (?1, 1), (?2, 1)",
        params![first_id, second_id],
    )?;
    drop(conn);

    let response = handle_graph(
        State(DbState),
        Query(GraphParams {
            project: Some("proj-a".to_string()),
            include_suppressed: None,
            limit: Some(10),
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    let nodes = payload["nodes"].as_array().expect("nodes should be array");
    let shared_node = nodes
        .iter()
        .find(|node| node["id"] == 1)
        .expect("shared entity should be returned");
    assert_eq!(
        shared_node["mems"],
        serde_json::json!([second_id, first_id])
    );
    Ok(())
}
