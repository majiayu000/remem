use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{header, Method, Request, StatusCode},
    response::IntoResponse,
};
use rusqlite::params;
use serde_json::Value;
use tower::ServiceExt;

use crate::db::test_support::ScopedTestDataDir;
use crate::{db, memory};

use super::handlers::{handle_status, search_request_from_params};
use super::types::SearchParams;
use super::DbState;

fn authorized_request(method: Method, uri: &str, token: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(body)
        .expect("request should build")
}

#[test]
fn db_state_is_stateless() {
    assert_eq!(std::mem::size_of::<DbState>(), 0);
}

#[test]
fn search_request_from_params_clamps_limit_and_offset() {
    let request = search_request_from_params(SearchParams {
        query: Some("hello".to_string()),
        project: None,
        memory_type: None,
        limit: Some(999),
        offset: Some(-5),
        include_stale: None,
        branch: None,
        multi_hop: None,
    });

    assert_eq!(request.limit, 100);
    assert_eq!(request.offset, 0);
    // Canonical default for `include_stale` is `true` so MCP and REST agree.
    assert!(request.include_stale);
    assert!(!request.multi_hop);
}

#[test]
fn search_request_from_params_preserves_filters() {
    let request = search_request_from_params(SearchParams {
        query: Some("hello".to_string()),
        project: Some("proj".to_string()),
        memory_type: Some("decision".to_string()),
        limit: Some(8),
        offset: Some(3),
        include_stale: Some(true),
        branch: Some("main".to_string()),
        multi_hop: Some(true),
    });

    assert_eq!(request.query.as_deref(), Some("hello"));
    assert_eq!(request.project.as_deref(), Some("proj"));
    assert_eq!(request.memory_type.as_deref(), Some("decision"));
    assert_eq!(request.limit, 8);
    assert_eq!(request.offset, 3);
    assert!(request.include_stale);
    assert_eq!(request.branch.as_deref(), Some("main"));
    assert!(request.multi_hop);
}

#[tokio::test]
async fn status_handler_reopens_database_after_file_removal() {
    let test_dir = ScopedTestDataDir::new("api-status");

    let first = handle_status(State(DbState)).await.into_response();
    assert_eq!(first.status(), StatusCode::OK);
    assert!(test_dir.db_path().exists());

    test_dir.remove_db_files();
    assert!(!test_dir.db_path().exists());

    let second = handle_status(State(DbState)).await.into_response();
    assert_eq!(second.status(), StatusCode::OK);
    assert!(test_dir.db_path().exists());
}

#[tokio::test]
async fn router_rejects_missing_and_invalid_api_token() {
    let _test_dir = ScopedTestDataDir::new("api-auth");
    crate::api::ensure_api_token().expect("API token should be created");
    let token = crate::api::load_api_token().expect("API token should load");
    let app = super::build_router(0).with_state(DbState);

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/memories")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"x","local_copy_enabled":false}"#))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

    let invalid = app
        .clone()
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/status",
            "wrong-token",
            Body::empty(),
        ))
        .await
        .expect("request should complete");
    assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);

    let valid = app
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/status",
            &token,
            Body::empty(),
        ))
        .await
        .expect("request should complete");
    assert_eq!(valid.status(), StatusCode::OK);
}

#[tokio::test]
async fn save_memory_response_reports_durable_feedback_shape() {
    let _test_dir = ScopedTestDataDir::new("api-save-feedback");
    crate::api::ensure_api_token().expect("API token should be created");
    let token = crate::api::load_api_token().expect("API token should load");
    let app = super::build_router(0).with_state(DbState);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/memories")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{
                        "text":"API durable feedback body",
                        "title":"API feedback",
                        "project":"proj",
                        "topic_key":"api-feedback",
                        "memory_type":"decision",
                        "scope":"project",
                        "branch":"main",
                        "local_copy_enabled":false
                    }"#,
                ))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should read");
    let payload: Value = serde_json::from_slice(&body).expect("save response should be valid json");

    assert_eq!(payload["status"], "saved");
    assert_eq!(payload["operation"], "add");
    assert_eq!(payload["upserted"], true);
    assert_eq!(payload["project"], "proj");
    assert_eq!(payload["scope"], "project");
    assert_eq!(payload["topic_key"], "api-feedback");
    assert_eq!(payload["branch"], "main");
    assert_eq!(payload["local_copy"]["status"], "disabled");
    assert_eq!(payload["local_status"], "disabled");
    assert!(payload["local_path"].is_null());
    assert_eq!(payload["claim_status"], "saved");
    assert!(payload["claim_id"].as_i64().is_some_and(|id| id > 0));
    assert!(payload["claim_error"].is_null());
    assert_eq!(payload["next_step"]["tool"], "get_observations");
    assert_eq!(payload["next_step"]["source"], "memory");
    assert_eq!(payload["next_step"]["ids"][0], payload["id"]);
    assert!(payload["created_at_epoch"]
        .as_i64()
        .is_some_and(|ts| ts > 0));
    assert!(payload["updated_at_epoch"]
        .as_i64()
        .is_some_and(|ts| ts > 0));
}

#[tokio::test]
async fn router_does_not_emit_cors_allow_origin_for_localhost_origin() {
    let _test_dir = ScopedTestDataDir::new("api-no-cors");
    crate::api::ensure_api_token().expect("API token should be created");
    let token = crate::api::load_api_token().expect("API token should load");
    let app = super::build_router(0).with_state(DbState);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/v1/status")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::ORIGIN, "http://localhost:3000")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .is_none());
}

#[tokio::test]
async fn status_handler_matches_shared_system_stats() {
    let _test_dir = ScopedTestDataDir::new("api-status-stats");
    let conn = db::open_db().expect("db should open");

    memory::insert_memory(
        &conn,
        Some("session-1"),
        "proj-a",
        None,
        "active memory",
        "kept",
        "decision",
        None,
    )
    .expect("active memory insert should succeed");
    let archived_memory_id = memory::insert_memory(
        &conn,
        Some("session-2"),
        "proj-a",
        None,
        "archived memory",
        "hidden",
        "decision",
        None,
    )
    .expect("archived memory insert should succeed");
    conn.execute(
        "UPDATE memories SET status = 'archived' WHERE id = ?1",
        params![archived_memory_id],
    )
    .expect("memory archive update should succeed");

    db::insert_observation(
        &conn,
        "session-1",
        "proj-a",
        "feature",
        Some("active observation"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        1,
    )
    .expect("active observation insert should succeed");
    let stale_observation_id = db::insert_observation(
        &conn,
        "session-2",
        "proj-a",
        "feature",
        Some("stale observation"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        1,
    )
    .expect("stale observation insert should succeed");
    conn.execute(
        "UPDATE observations SET status = 'stale' WHERE id = ?1",
        params![stale_observation_id],
    )
    .expect("observation stale update should succeed");

    let stats = db::query_system_stats(&conn).expect("system stats should load");
    drop(conn);

    let response = handle_status(State(DbState)).await.into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should read");
    let payload: Value =
        serde_json::from_slice(&body).expect("status response should be valid json");
    assert_eq!(payload["memories"], stats.active_memories);
    assert_eq!(payload["observations"], stats.active_observations);
    assert_eq!(payload["captured_events"], stats.captured_events);
    assert_eq!(
        payload["pending_extraction_tasks"],
        stats.pending_extraction_tasks
    );
    assert_eq!(
        payload["pending_graph_candidates"],
        stats.pending_graph_candidates
    );
}
