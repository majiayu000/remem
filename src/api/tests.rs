use axum::{
    body::{to_bytes, Body},
    extract::{Path, Query, State},
    http::{header, Method, Request, StatusCode},
    response::IntoResponse,
    Extension,
};
use rusqlite::params;
use serde_json::Value;
use tower::ServiceExt;

use crate::db::test_support::ScopedTestDataDir;
use crate::{db, memory};

use super::handlers::{
    handle_get_memory, handle_graph, handle_list_memories, handle_memory_detail, handle_search,
    handle_stats, handle_status, search_request_from_params,
};
use super::types::{
    GraphParams, ListParams, MemoryDetailParams, SearchParams, ShowParams, StatusCache,
    StatusParams,
};
use super::DbState;

mod candidates;
mod web_regressions;

fn authorized_request(method: Method, uri: &str, token: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(body)
        .expect("request should build")
}

fn authorized_json_request(method: Method, uri: &str, token: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request should build")
}

fn insert_review_candidate(topic_key: &str, text: &str) -> anyhow::Result<i64> {
    let mut conn = db::open_db()?;
    db::record_captured_event(
        &conn,
        &db::CaptureEventInput {
            host: "codex-cli",
            session_id: "api-candidate-review",
            project: "proj-review",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: "candidate review API fixture evidence",
            task_kind: Some(db::ExtractionTaskKind::MemoryCandidate),
        },
    )?;
    let task = db::claim_next_extraction_task(&mut conn, "api-candidate-review-worker", 60)?
        .ok_or_else(|| anyhow::anyhow!("candidate review fixture task should claim"))?;
    let evidence_event_ids = serde_json::to_string(&vec![task
        .high_watermark_event_id
        .ok_or_else(|| anyhow::anyhow!("fixture task should have high watermark"))?])?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO memory_candidates
          (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
           confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES
          (?1, 'project', 'decision', ?2, ?3, ?4, 0.82,
           'medium', 'pending_review', ?5, ?5)",
        params![task.project_id, topic_key, text, evidence_event_ids, now],
    )?;
    Ok(conn.last_insert_rowid())
}

fn default_status_extractors() -> (State<DbState>, Query<StatusParams>, Extension<StatusCache>) {
    (
        State(DbState),
        Query(StatusParams::default()),
        Extension(StatusCache::default()),
    )
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
        include_suppressed: None,
        branch: None,
        multi_hop: None,
        explain: None,
    });

    assert_eq!(request.limit, 100);
    assert_eq!(request.offset, 0);
    // Canonical default hides stale and archived memories unless callers opt in.
    assert!(!request.include_stale);
    assert!(!request.include_suppressed);
    assert!(!request.multi_hop);
    assert!(!request.explain);
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
        include_suppressed: Some(true),
        branch: Some("main".to_string()),
        multi_hop: Some(true),
        explain: Some(true),
    });

    assert_eq!(request.query.as_deref(), Some("hello"));
    assert_eq!(request.project.as_deref(), Some("proj"));
    assert_eq!(request.memory_type.as_deref(), Some("decision"));
    assert_eq!(request.limit, 8);
    assert_eq!(request.offset, 3);
    assert!(request.include_suppressed);
    assert!(request.include_stale);
    assert_eq!(request.branch.as_deref(), Some("main"));
    assert!(request.multi_hop);
    assert!(request.explain);
}

#[tokio::test]
async fn search_handler_hides_inactive_memories_by_default() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-search-default-active");
    let conn = db::open_db()?;

    memory::insert_memory(
        &conn,
        Some("session-active"),
        "proj-a",
        None,
        "aurora active",
        "aurora visible memory",
        "decision",
        None,
    )?;
    let stale_id = memory::insert_memory(
        &conn,
        Some("session-stale"),
        "proj-a",
        None,
        "aurora stale",
        "aurora stale memory",
        "decision",
        None,
    )?;
    let archived_id = memory::insert_memory(
        &conn,
        Some("session-archived"),
        "proj-a",
        None,
        "aurora archived",
        "aurora hidden memory",
        "decision",
        None,
    )?;
    conn.execute(
        "UPDATE memories SET status = 'stale' WHERE id = ?1",
        params![stale_id],
    )?;
    conn.execute(
        "UPDATE memories SET status = 'archived' WHERE id = ?1",
        params![archived_id],
    )?;
    drop(conn);

    let response = handle_search(
        State(DbState),
        axum::extract::Query(SearchParams {
            query: Some("aurora".to_string()),
            project: Some("proj-a".to_string()),
            memory_type: None,
            limit: Some(10),
            offset: None,
            include_stale: None,
            include_suppressed: None,
            branch: None,
            multi_hop: None,
            explain: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    let data = payload["data"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("search response data should be an array"))?;
    let titles: Vec<&str> = data
        .iter()
        .map(|item| {
            item["title"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("search item title should be a string"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    assert_eq!(titles, vec!["aurora active"]);
    Ok(())
}

#[tokio::test]
async fn get_memory_handler_marks_memory_accessed() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-get-memory-usage");
    let conn = db::open_db()?;
    let memory_id = memory::insert_memory(
        &conn,
        Some("session-usage"),
        "proj-a",
        None,
        "usage target",
        "single-memory API detail reads should update usage columns",
        "decision",
        None,
    )?;
    drop(conn);

    let response = handle_get_memory(
        State(DbState),
        axum::extract::Query(ShowParams {
            id: memory_id,
            include_suppressed: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let conn = db::open_db()?;
    let usage: (i64, Option<i64>) = conn.query_row(
        "SELECT access_count, last_accessed_epoch FROM memories WHERE id = ?1",
        [memory_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(usage.0, 1);
    assert!(usage.1.is_some());
    Ok(())
}

#[tokio::test]
async fn get_memory_handler_hides_policy_suppressed_rows_by_default() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-get-memory-policy-suppressed");
    let conn = db::open_db()?;
    let memory_id = memory::insert_memory(
        &conn,
        Some("session-legacy-detail-policy"),
        "proj-a",
        None,
        "hidden legacy detail",
        "legacy detail should require include_suppressed",
        "decision",
        None,
    )?;
    crate::memory::suppression::create_suppression(
        &conn,
        &crate::memory::suppression::SuppressRequest {
            target: crate::memory::suppression::parse_target(&format!("memory:{memory_id}"))?,
            reason: Some("not useful"),
            actor: Some("test"),
        },
    )?;
    drop(conn);

    let default_response = handle_get_memory(
        State(DbState),
        Query(ShowParams {
            id: memory_id,
            include_suppressed: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(default_response.status(), StatusCode::NOT_FOUND);

    let audit_response = handle_get_memory(
        State(DbState),
        Query(ShowParams {
            id: memory_id,
            include_suppressed: Some(true),
        }),
    )
    .await
    .into_response();
    assert_eq!(audit_response.status(), StatusCode::OK);
    let body = to_bytes(audit_response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["id"], memory_id);
    Ok(())
}

#[tokio::test]
async fn status_handler_reopens_database_after_file_removal() {
    let test_dir = ScopedTestDataDir::new("api-status");
    let cache = StatusCache::default();

    let first = handle_status(
        State(DbState),
        Query(StatusParams::default()),
        Extension(cache.clone()),
    )
    .await
    .into_response();
    assert_eq!(first.status(), StatusCode::OK);
    assert!(test_dir.db_path().exists());

    test_dir.remove_db_files();
    assert!(!test_dir.db_path().exists());

    let second = handle_status(
        State(DbState),
        Query(StatusParams {
            refresh: Some(true),
        }),
        Extension(cache),
    )
    .await
    .into_response();
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
async fn router_serves_capabilities_with_auth() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-capabilities");
    crate::api::ensure_api_token().expect("API token should be created");
    let token = crate::api::load_api_token().expect("API token should load");
    let app = super::build_router(0).with_state(DbState);

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/v1/capabilities")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/capabilities",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;

    assert_eq!(payload["version"], crate::build_info::package_version());
    assert_eq!(
        payload["schema_version"],
        crate::build_info::binary_schema_version()
    );
    assert_eq!(payload["api_version"], 1);
    assert_eq!(payload["features"]["health"], true);
    assert_eq!(payload["features"]["status"], true);
    assert_eq!(payload["features"]["stats"], true);
    assert_eq!(payload["features"]["search"], true);
    assert_eq!(payload["features"]["search_explain"], true);
    assert_eq!(payload["features"]["memory_list"], true);
    assert_eq!(payload["features"]["memory_detail"], true);
    assert_eq!(payload["features"]["save_memory"], true);
    assert_eq!(payload["features"]["candidate_rows"], true);
    assert_eq!(payload["features"]["candidate_review"], true);
    assert_eq!(payload["features"]["graph"], true);
    assert_eq!(payload["features"]["user_recall"], true);
    assert_eq!(payload["features"]["user_recall_usage_policy"], true);
    assert_eq!(payload["endpoints"]["health"], "/api/v1/health");
    assert_eq!(payload["endpoints"]["status"], "/api/v1/status");
    assert_eq!(payload["endpoints"]["stats"], "/api/v1/stats");
    assert_eq!(payload["endpoints"]["search"], "/api/v1/search");
    assert_eq!(
        payload["endpoints"]["search_explain"],
        "/api/v1/search?explain=true"
    );
    assert_eq!(payload["endpoints"]["memory_list"], "/api/v1/memories");
    assert_eq!(
        payload["endpoints"]["memory_detail"],
        "/api/v1/memories/{id}"
    );
    assert_eq!(payload["endpoints"]["save_memory"], "/api/v1/memories");
    assert_eq!(payload["endpoints"]["candidate_rows"], "/api/v1/candidates");
    assert_eq!(
        payload["endpoints"]["candidate_blocked"],
        "/api/v1/candidates/blocked"
    );
    assert_eq!(
        payload["endpoints"]["candidate_review"],
        "/api/v1/candidates/{id}/approve"
    );
    assert_eq!(payload["endpoints"]["graph"], "/api/v1/graph");
    assert_eq!(payload["endpoints"]["user_recall"], "/api/v1/user/recall");
    assert!(payload.get("token").is_none());

    Ok(())
}

#[tokio::test]
async fn router_serves_user_recall_with_auth() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-user-recall");
    crate::api::ensure_api_token().expect("API token should be created");
    let token = crate::api::load_api_token().expect("API token should load");
    let conn = db::open_db()?;
    crate::user_context::claims::create_manual_claim(
        &conn,
        &crate::user_context::claims::ManualClaimRequest {
            text: "Prefer API recall JSON",
            owner_scope: None,
            owner_key: None,
            claim_type: crate::user_context::claims::UserContextClaimType::Preference,
            claim_key: None,
            confidence: 1.0,
            sensitivity: crate::user_context::claims::UserContextSensitivity::Normal,
            valid_from_epoch: None,
            valid_to_epoch: None,
        },
    )?;
    drop(conn);
    let app = super::build_router(0).with_state(DbState);

    let response = app
        .oneshot(authorized_json_request(
            Method::POST,
            "/api/v1/user/recall",
            &token,
            r#"{"query":"API recall","project":"/repo","limit":5,"budget_chars":1000}"#,
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["empty"], false);
    assert_eq!(
        payload["usage_policy"],
        crate::user_context::usage_policy::USER_CONTEXT_USAGE_POLICY
    );
    assert_eq!(payload["included"][0]["source_type"], "user_claim");
    assert!(payload["context"]
        .as_str()
        .unwrap_or_default()
        .contains("recall"));
    assert!(!payload["context"]
        .as_str()
        .unwrap_or_default()
        .contains(crate::user_context::usage_policy::USER_CONTEXT_USAGE_POLICY));
    Ok(())
}

#[tokio::test]
async fn router_serves_health_with_auth_without_opening_database() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("api-health");
    crate::api::ensure_api_token().expect("API token should be created");
    let token = crate::api::load_api_token().expect("API token should load");
    let app = super::build_router(0).with_state(DbState);

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/v1/health")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/health",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["version"], crate::build_info::package_version());
    assert_eq!(payload["api_version"], 1);
    assert_eq!(
        payload["schema_version"],
        crate::build_info::binary_schema_version()
    );
    assert!(payload.get("token").is_none());
    assert!(payload.get("path").is_none());
    assert!(!test_dir.db_path().exists());

    Ok(())
}

#[tokio::test]
async fn status_router_reports_cache_hits_and_refresh() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-status-cache");
    crate::api::ensure_api_token().expect("API token should be created");
    let token = crate::api::load_api_token().expect("API token should load");
    let app = super::build_router(0).with_state(DbState);

    let first = app
        .clone()
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/status",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(first.status(), StatusCode::OK);
    let first_body = to_bytes(first.into_body(), usize::MAX).await?;
    let first_payload: Value = serde_json::from_slice(&first_body)?;
    assert_eq!(first_payload["cache"]["hit"], false);
    assert_eq!(first_payload["cache"]["stale"], false);
    assert_eq!(first_payload["cache"]["ttl_secs"], 2);
    assert!(first_payload["warnings"].is_null());
    let first_memories = first_payload["memories"]
        .as_i64()
        .expect("status memories should be numeric");

    let cached = app
        .clone()
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/status",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(cached.status(), StatusCode::OK);
    let cached_body = to_bytes(cached.into_body(), usize::MAX).await?;
    let cached_payload: Value = serde_json::from_slice(&cached_body)?;
    assert_eq!(cached_payload["cache"]["hit"], true);
    assert_eq!(cached_payload["cache"]["stale"], false);
    assert_eq!(cached_payload["memories"], first_memories);

    let conn = db::open_db()?;
    memory::insert_memory(
        &conn,
        Some("session-cache"),
        "proj-cache",
        None,
        "status cache memory",
        "visible after refresh",
        "decision",
        None,
    )?;
    drop(conn);

    let refreshed = app
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/status?refresh=true",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(refreshed.status(), StatusCode::OK);
    let refreshed_body = to_bytes(refreshed.into_body(), usize::MAX).await?;
    let refreshed_payload: Value = serde_json::from_slice(&refreshed_body)?;
    assert_eq!(refreshed_payload["cache"]["hit"], false);
    assert_eq!(refreshed_payload["cache"]["stale"], false);
    assert_eq!(refreshed_payload["memories"], first_memories + 1);

    Ok(())
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
                        "created_at_epoch":1700000789,
                        "reference_time_epoch":1600000456,
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
    assert_eq!(payload["created_at_epoch"], 1_700_000_789);
    assert_eq!(payload["reference_time_epoch"], 1_600_000_456);
    assert!(payload["updated_at_epoch"]
        .as_i64()
        .is_some_and(|ts| ts > 0));
}

#[tokio::test]
async fn router_serves_get_api_v1_memories() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-list-route");
    let conn = db::open_db()?;
    memory::insert_memory(
        &conn,
        Some("session-list-route"),
        "proj-a",
        None,
        "route target",
        "documented list route should serve this memory",
        "decision",
        None,
    )?;
    drop(conn);

    crate::api::ensure_api_token().expect("API token should be created");
    let token = crate::api::load_api_token().expect("token should load");
    let app = super::build_router(0).with_state(DbState);

    let response = app
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/memories?project=proj-a",
            &token,
            Body::empty(),
        ))
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["meta"]["total"], 1);
    assert_eq!(payload["meta"]["count"], 1);
    assert_eq!(payload["meta"]["limit"], 50);
    assert_eq!(payload["meta"]["offset"], 0);
    assert_eq!(payload["meta"]["has_more"], false);
    assert!(payload["meta"]["next_offset"].is_null());
    assert_eq!(payload["data"][0]["title"], "route target");
    Ok(())
}

#[tokio::test]
async fn list_memories_excludes_policy_suppressed_rows_by_default() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-list-policy-suppressed");
    let conn = db::open_db()?;
    let visible_id = memory::insert_memory(
        &conn,
        Some("session-list-policy-visible"),
        "proj-a",
        None,
        "visible list row",
        "visible list memory",
        "decision",
        None,
    )?;
    let hidden_id = memory::insert_memory(
        &conn,
        Some("session-list-policy-hidden"),
        "proj-a",
        None,
        "hidden list row",
        "hidden list memory",
        "decision",
        None,
    )?;
    crate::memory::suppression::create_suppression(
        &conn,
        &crate::memory::suppression::SuppressRequest {
            target: crate::memory::suppression::parse_target(&format!("memory:{hidden_id}"))?,
            reason: Some("not useful"),
            actor: Some("test"),
        },
    )?;
    drop(conn);

    let default_response = handle_list_memories(
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
    assert_eq!(default_response.status(), StatusCode::OK);
    let default_body = to_bytes(default_response.into_body(), usize::MAX).await?;
    let default_payload: Value = serde_json::from_slice(&default_body)?;
    assert_eq!(default_payload["meta"]["total"], 1);
    assert_eq!(default_payload["data"][0]["id"], visible_id);

    let audit_response = handle_list_memories(
        State(DbState),
        Query(ListParams {
            project: Some("proj-a".to_string()),
            memory_type: None,
            scope: None,
            status: None,
            branch: None,
            q: None,
            include_suppressed: Some(true),
            limit: Some(10),
            offset: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(audit_response.status(), StatusCode::OK);
    let audit_body = to_bytes(audit_response.into_body(), usize::MAX).await?;
    let audit_payload: Value = serde_json::from_slice(&audit_body)?;
    let ids: Vec<i64> = audit_payload["data"]
        .as_array()
        .expect("data should be array")
        .iter()
        .map(|item| item["id"].as_i64().expect("id should be i64"))
        .collect();
    assert!(ids.contains(&visible_id));
    assert!(ids.contains(&hidden_id));
    Ok(())
}

#[tokio::test]
async fn router_memories_and_list_alias_return_same_pagination_meta() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-list-alias-meta");
    let conn = db::open_db()?;
    for i in 0..3 {
        memory::insert_memory(
            &conn,
            Some("session-list-alias"),
            "proj-a",
            None,
            &format!("page target {i}"),
            "canonical and alias routes should page the same way",
            "decision",
            None,
        )?;
    }
    drop(conn);

    crate::api::ensure_api_token().expect("API token should be created");
    let token = crate::api::load_api_token().expect("token should load");
    let app = super::build_router(0).with_state(DbState);

    let canonical = app
        .clone()
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/memories?project=proj-a&limit=2&offset=0",
            &token,
            Body::empty(),
        ))
        .await
        .expect("request should complete");
    assert_eq!(canonical.status(), StatusCode::OK);

    let alias = app
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/memories/list?project=proj-a&limit=2&offset=0",
            &token,
            Body::empty(),
        ))
        .await
        .expect("request should complete");
    assert_eq!(alias.status(), StatusCode::OK);

    let canonical_body = to_bytes(canonical.into_body(), usize::MAX).await?;
    let alias_body = to_bytes(alias.into_body(), usize::MAX).await?;
    let canonical_payload: Value = serde_json::from_slice(&canonical_body)?;
    let alias_payload: Value = serde_json::from_slice(&alias_body)?;

    assert_eq!(canonical_payload["meta"], alias_payload["meta"]);
    assert_eq!(canonical_payload["meta"]["count"], 2);
    assert_eq!(canonical_payload["meta"]["total"], 3);
    assert_eq!(canonical_payload["meta"]["limit"], 2);
    assert_eq!(canonical_payload["meta"]["offset"], 0);
    assert_eq!(canonical_payload["meta"]["has_more"], true);
    assert_eq!(canonical_payload["meta"]["next_offset"], 2);
    Ok(())
}

#[tokio::test]
async fn list_memories_q_filter_matches_title_or_content() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-list-q-filter");
    let conn = db::open_db()?;
    let title_match = memory::insert_memory(
        &conn,
        Some("session-q-title"),
        "proj-a",
        None,
        "needle title",
        "ordinary body",
        "decision",
        None,
    )?;
    let content_match = memory::insert_memory(
        &conn,
        Some("session-q-content"),
        "proj-a",
        None,
        "ordinary title",
        "body has needle",
        "decision",
        None,
    )?;
    let other = memory::insert_memory(
        &conn,
        Some("session-q-other"),
        "proj-a",
        None,
        "unrelated",
        "ordinary body",
        "decision",
        None,
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
            q: Some("needle".to_string()),
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
    assert!(ids.contains(&title_match));
    assert!(ids.contains(&content_match));
    assert!(!ids.contains(&other));
    assert_eq!(payload["meta"]["total"], 2);
    Ok(())
}

#[tokio::test]
async fn list_memories_branch_filter_includes_null_branch() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-list-branch-null");
    let conn = db::open_db()?;
    let branchless = memory::insert_memory(
        &conn,
        Some("session-branchless"),
        "proj-a",
        None,
        "branchless",
        "branch-agnostic memory",
        "decision",
        None,
    )?;
    let main_branch = memory::insert_memory_with_branch(
        &conn,
        Some("session-main"),
        "proj-a",
        None,
        "main branch",
        "main branch memory",
        "decision",
        None,
        Some("main"),
    )?;
    let other_branch = memory::insert_memory_with_branch(
        &conn,
        Some("session-other"),
        "proj-a",
        None,
        "other branch",
        "other branch memory",
        "decision",
        None,
        Some("other"),
    )?;
    drop(conn);

    let response = handle_list_memories(
        State(DbState),
        Query(ListParams {
            project: Some("proj-a".to_string()),
            memory_type: None,
            scope: None,
            status: None,
            branch: Some("main".to_string()),
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
    assert!(ids.contains(&branchless));
    assert!(ids.contains(&main_branch));
    assert!(!ids.contains(&other_branch));
    assert_eq!(payload["meta"]["total"], 2);
    Ok(())
}

#[tokio::test]
async fn list_memories_active_status_excludes_expired_rows() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-list-active-current");
    let conn = db::open_db()?;
    let current_id = memory::insert_memory(
        &conn,
        Some("session-current-list"),
        "proj-a",
        None,
        "current list row",
        "current active memory",
        "decision",
        None,
    )?;
    let expired_id = memory::insert_memory(
        &conn,
        Some("session-expired-list"),
        "proj-a",
        None,
        "expired list row",
        "expired active memory",
        "decision",
        None,
    )?;
    conn.execute(
        "UPDATE memories SET expires_at_epoch = 1 WHERE id = ?1",
        params![expired_id],
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
    assert!(!ids.contains(&expired_id));
    assert_eq!(payload["meta"]["total"], 1);
    Ok(())
}

#[tokio::test]
async fn router_approves_candidate_review() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidate-approve");
    let candidate_id = insert_review_candidate("api-approve", "Approve through API")?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::build_router(0).with_state(DbState);

    let response = app
        .oneshot(authorized_request(
            Method::POST,
            &format!("/api/v1/candidates/{candidate_id}/approve"),
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["candidate_id"], candidate_id);
    assert_eq!(payload["status"], "approved");
    let memory_id = payload["memory_id"]
        .as_i64()
        .ok_or_else(|| anyhow::anyhow!("approve response should include memory_id"))?;

    let conn = db::open_db()?;
    let source_candidate_id: i64 = conn.query_row(
        "SELECT source_candidate_id FROM memories WHERE id = ?1",
        params![memory_id],
        |row| row.get(0),
    )?;
    assert_eq!(source_candidate_id, candidate_id);
    Ok(())
}

#[tokio::test]
async fn router_rejects_candidate_review() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidate-reject");
    let candidate_id = insert_review_candidate("api-reject", "Reject through API")?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::build_router(0).with_state(DbState);

    let response = app
        .oneshot(authorized_request(
            Method::POST,
            &format!("/api/v1/candidates/{candidate_id}/reject"),
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["candidate_id"], candidate_id);
    assert_eq!(payload["status"], "discarded");
    assert!(payload.get("memory_id").is_none());

    let conn = db::open_db()?;
    let status: String = conn.query_row(
        "SELECT review_status FROM memory_candidates WHERE id = ?1",
        params![candidate_id],
        |row| row.get(0),
    )?;
    assert_eq!(status, "discarded");
    Ok(())
}

#[tokio::test]
async fn router_edits_candidate_review() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidate-edit");
    let candidate_id = insert_review_candidate("api-edit", "Original candidate text")?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::build_router(0).with_state(DbState);

    let response = app
        .oneshot(authorized_json_request(
            Method::POST,
            &format!("/api/v1/candidates/{candidate_id}/edit"),
            &token,
            r#"{
                "scope":"project",
                "memory_type":"architecture",
                "topic_key":"api-edited",
                "text":"Edited through the API"
            }"#,
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["candidate_id"], candidate_id);
    assert_eq!(payload["status"], "edited");
    let memory_id = payload["memory_id"]
        .as_i64()
        .ok_or_else(|| anyhow::anyhow!("edit response should include memory_id"))?;

    let conn = db::open_db()?;
    let (review_status, memory_type, topic_key, content): (String, String, String, String) = conn
        .query_row(
        "SELECT c.review_status, m.memory_type, m.topic_key, m.content
             FROM memory_candidates c
             JOIN memories m ON m.id = ?2
             WHERE c.id = ?1",
        params![candidate_id, memory_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(review_status, "edited");
    assert_eq!(memory_type, "architecture");
    assert_eq!(topic_key, "api-edited");
    assert_eq!(content, "Edited through the API");
    Ok(())
}

#[tokio::test]
async fn router_candidate_review_reports_not_found() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidate-not-found");
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::build_router(0).with_state(DbState);

    let response = app
        .oneshot(authorized_request(
            Method::POST,
            "/api/v1/candidates/404/approve",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["error"]["code"], "not_found");
    assert_eq!(payload["error"]["message"], "candidate 404 not found");
    Ok(())
}

#[tokio::test]
async fn router_candidate_review_reports_non_pending_conflict() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidate-non-pending");
    let candidate_id = insert_review_candidate("api-non-pending", "Already approved")?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::build_router(0).with_state(DbState);

    let first = app
        .clone()
        .oneshot(authorized_request(
            Method::POST,
            &format!("/api/v1/candidates/{candidate_id}/approve"),
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(first.status(), StatusCode::OK);

    let second = app
        .oneshot(authorized_request(
            Method::POST,
            &format!("/api/v1/candidates/{candidate_id}/reject"),
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(second.status(), StatusCode::CONFLICT);

    let body = to_bytes(second.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["error"]["code"], "candidate_not_pending");
    assert_eq!(
        payload["error"]["message"],
        format!("candidate {candidate_id} is already approved")
    );
    Ok(())
}

#[tokio::test]
async fn router_candidate_review_reports_invalid_edit() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidate-invalid-edit");
    let candidate_id = insert_review_candidate("api-invalid-edit", "Invalid edit")?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::build_router(0).with_state(DbState);

    let empty = app
        .clone()
        .oneshot(authorized_json_request(
            Method::POST,
            &format!("/api/v1/candidates/{candidate_id}/edit"),
            &token,
            "{}",
        ))
        .await?;
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
    let empty_body = to_bytes(empty.into_body(), usize::MAX).await?;
    let empty_payload: Value = serde_json::from_slice(&empty_body)?;
    assert_eq!(empty_payload["error"]["code"], "candidate_edit_invalid");

    let invalid_type = app
        .oneshot(authorized_json_request(
            Method::POST,
            &format!("/api/v1/candidates/{candidate_id}/edit"),
            &token,
            r#"{"memory_type":"session_activity"}"#,
        ))
        .await?;
    assert_eq!(invalid_type.status(), StatusCode::BAD_REQUEST);
    let invalid_body = to_bytes(invalid_type.into_body(), usize::MAX).await?;
    let invalid_payload: Value = serde_json::from_slice(&invalid_body)?;
    assert_eq!(invalid_payload["error"]["code"], "candidate_edit_invalid");
    Ok(())
}

#[tokio::test]
async fn router_candidate_review_rolls_back_failed_promotion() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-candidate-review-rollback");
    let candidate_id = insert_review_candidate("api-rollback", "Rollback failed promotion")?;
    let conn = db::open_db()?;
    conn.execute(
        "UPDATE memory_candidates SET confidence = 1.5 WHERE id = ?1",
        params![candidate_id],
    )?;
    drop(conn);

    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::build_router(0).with_state(DbState);

    let response = app
        .oneshot(authorized_request(
            Method::POST,
            &format!("/api/v1/candidates/{candidate_id}/approve"),
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["error"]["code"], "candidate_review_failed");
    assert!(!payload["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .is_empty());

    let conn = db::open_db()?;
    let review_status: String = conn.query_row(
        "SELECT review_status FROM memory_candidates WHERE id = ?1",
        params![candidate_id],
        |row| row.get(0),
    )?;
    assert_eq!(review_status, "pending_review");
    let memory_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE source_candidate_id = ?1",
        params![candidate_id],
        |row| row.get(0),
    )?;
    assert_eq!(memory_count, 0);
    Ok(())
}

#[tokio::test]
async fn memory_detail_returns_rich_memory_with_entities() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-memory-detail-rich");
    let conn = db::open_db()?;
    let memory_id = memory::insert_memory(
        &conn,
        Some("session-detail"),
        "proj-a",
        Some("detail-topic"),
        "detail title",
        "detail body",
        "decision",
        None,
    )?;
    conn.execute(
        "INSERT INTO entities (id, canonical_name, entity_type, mention_count, created_at_epoch)
         VALUES (1, 'api', 'topic', 5, 1), (2, 'sqlcipher', 'topic', 9, 1)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_entities (memory_id, entity_id) VALUES (?1, 1), (?1, 2)",
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
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["id"], memory_id);
    assert_eq!(payload["title"], "detail title");
    assert_eq!(payload["content"], "detail body");
    assert_eq!(payload["memory_type"], "decision");
    assert_eq!(payload["project"], "proj-a");
    assert_eq!(payload["topic_key"], "detail-topic");
    assert_eq!(payload["entities"], serde_json::json!(["sqlcipher", "api"]));
    assert_eq!(
        payload["edges"]
            .as_array()
            .expect("edges should be array")
            .len(),
        0
    );
    Ok(())
}

#[tokio::test]
async fn memory_detail_hides_policy_suppressed_rows_by_default() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-memory-detail-policy-suppressed");
    let conn = db::open_db()?;
    let memory_id = memory::insert_memory(
        &conn,
        Some("session-rich-detail-policy"),
        "proj-a",
        None,
        "hidden rich detail",
        "rich detail should require include_suppressed",
        "decision",
        None,
    )?;
    crate::memory::suppression::create_suppression(
        &conn,
        &crate::memory::suppression::SuppressRequest {
            target: crate::memory::suppression::parse_target(&format!("memory:{memory_id}"))?,
            reason: Some("not useful"),
            actor: Some("test"),
        },
    )?;
    drop(conn);

    let default_response = handle_memory_detail(
        State(DbState),
        Path(memory_id),
        Query(MemoryDetailParams {
            include_suppressed: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(default_response.status(), StatusCode::NOT_FOUND);

    let audit_response = handle_memory_detail(
        State(DbState),
        Path(memory_id),
        Query(MemoryDetailParams {
            include_suppressed: Some(true),
        }),
    )
    .await
    .into_response();
    assert_eq!(audit_response.status(), StatusCode::OK);
    let body = to_bytes(audit_response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["id"], memory_id);
    Ok(())
}

#[tokio::test]
async fn memory_detail_returns_structured_not_found() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-memory-detail-not-found");

    let response = handle_memory_detail(
        State(DbState),
        Path(404),
        Query(MemoryDetailParams {
            include_suppressed: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["error"]["code"], "not_found");
    assert_eq!(payload["error"]["message"], "memory 404 not found");
    Ok(())
}

#[tokio::test]
async fn memory_detail_includes_incoming_and_outgoing_edges() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-memory-detail-edges");
    let conn = db::open_db()?;
    let old_id = memory::insert_memory(
        &conn,
        Some("session-old"),
        "proj-a",
        Some("edge-old"),
        "old",
        "old memory",
        "decision",
        None,
    )?;
    let new_id = memory::insert_memory(
        &conn,
        Some("session-new"),
        "proj-a",
        Some("edge-new"),
        "new",
        "new memory",
        "decision",
        None,
    )?;
    conn.execute(
        "INSERT INTO memory_edges
          (edge_type, from_memory_id, to_memory_id, confidence, created_at_epoch)
         VALUES ('replaces', ?1, ?2, 0.9, 1)",
        params![old_id, new_id],
    )?;
    conn.execute(
        "INSERT INTO memory_edges
          (edge_type, from_memory_id, to_memory_id, confidence, created_at_epoch)
         VALUES ('derived_from', ?1, ?2, 0.8, 2)",
        params![new_id, old_id],
    )?;
    drop(conn);

    let response = handle_memory_detail(
        State(DbState),
        Path(new_id),
        Query(MemoryDetailParams {
            include_suppressed: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    let edges = payload["edges"].as_array().expect("edges should be array");
    assert_eq!(edges.len(), 2);
    assert!(edges.iter().any(|edge| {
        edge["edge_type"] == "replaces"
            && edge["from_memory_id"] == old_id
            && edge["to_memory_id"] == new_id
    }));
    assert!(edges.iter().any(|edge| {
        edge["edge_type"] == "derived_from"
            && edge["from_memory_id"] == new_id
            && edge["to_memory_id"] == old_id
    }));
    Ok(())
}

#[tokio::test]
async fn graph_limits_memory_fanout_per_entity() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-graph-fanout");
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO entities (id, canonical_name, entity_type, mention_count, created_at_epoch)
         VALUES (1, 'high degree', 'topic', 999, 1)",
        [],
    )?;
    for i in 1..=205 {
        let memory_id = memory::insert_memory(
            &conn,
            Some("session-graph"),
            "proj-a",
            None,
            &format!("graph memory {i}"),
            "graph memory fanout fixture",
            "decision",
            None,
        )?;
        conn.execute(
            "INSERT INTO memory_entities (memory_id, entity_id) VALUES (?1, 1)",
            params![memory_id],
        )?;
    }
    conn.execute("UPDATE memories SET state_key_id = NULL", [])?;
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
    assert_eq!(payload["nodes"][0]["mems"].as_array().unwrap().len(), 200);
    Ok(())
}

#[tokio::test]
async fn graph_uses_only_current_memories_for_links() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-graph-current-memories");
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO entities (id, canonical_name, entity_type, mention_count, created_at_epoch)
         VALUES
          (1, 'left', 'topic', 10, 1),
          (2, 'right', 'topic', 9, 1)",
        [],
    )?;
    let current_id = memory::insert_memory(
        &conn,
        Some("session-current-graph"),
        "proj-a",
        None,
        "current graph row",
        "current graph memory",
        "decision",
        None,
    )?;
    let expired_id = memory::insert_memory(
        &conn,
        Some("session-expired-graph"),
        "proj-a",
        None,
        "expired graph row",
        "expired graph memory",
        "decision",
        None,
    )?;
    let stale_id = memory::insert_memory(
        &conn,
        Some("session-stale-graph"),
        "proj-a",
        None,
        "stale graph row",
        "stale graph memory",
        "decision",
        None,
    )?;
    conn.execute(
        "UPDATE memories SET expires_at_epoch = 1 WHERE id = ?1",
        params![expired_id],
    )?;
    conn.execute(
        "UPDATE memories SET status = 'stale' WHERE id = ?1",
        params![stale_id],
    )?;
    conn.execute(
        "INSERT INTO memory_entities (memory_id, entity_id) VALUES
         (?1, 1), (?1, 2),
         (?2, 1), (?2, 2),
         (?3, 1)",
        params![current_id, expired_id, stale_id],
    )?;
    drop(conn);

    let response = handle_graph(
        State(DbState),
        Query(GraphParams {
            project: None,
            include_suppressed: None,
            limit: Some(2),
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    let left_node = payload["nodes"]
        .as_array()
        .expect("nodes should be array")
        .iter()
        .find(|node| node["id"] == 1)
        .expect("left node should exist");
    let left_mems: Vec<i64> = left_node["mems"]
        .as_array()
        .expect("mems should be array")
        .iter()
        .map(|id| id.as_i64().expect("memory id should be i64"))
        .collect();
    assert_eq!(left_mems, vec![current_id]);
    assert!(!left_mems.contains(&expired_id));
    assert!(!left_mems.contains(&stale_id));
    assert_eq!(payload["edges"][0]["w"], 1);
    Ok(())
}

#[tokio::test]
async fn graph_excludes_policy_suppressed_memories_by_default() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-graph-policy-suppressed");
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO entities (id, canonical_name, entity_type, mention_count, created_at_epoch)
         VALUES
          (1, 'hidden-only', 'topic', 1, 1),
          (2, 'visible-only', 'topic', 1, 1)",
        [],
    )?;
    let hidden_id = memory::insert_memory(
        &conn,
        Some("session-hidden-graph-policy"),
        "proj-a",
        None,
        "hidden graph row",
        "hidden graph memory",
        "decision",
        None,
    )?;
    let visible_id = memory::insert_memory(
        &conn,
        Some("session-visible-graph-policy"),
        "proj-a",
        None,
        "visible graph row",
        "visible graph memory",
        "decision",
        None,
    )?;
    conn.execute(
        "INSERT INTO memory_entities (memory_id, entity_id) VALUES (?1, 1), (?2, 2)",
        params![hidden_id, visible_id],
    )?;
    crate::memory::suppression::create_suppression(
        &conn,
        &crate::memory::suppression::SuppressRequest {
            target: crate::memory::suppression::parse_target(&format!("memory:{hidden_id}"))?,
            reason: Some("not useful"),
            actor: Some("test"),
        },
    )?;
    drop(conn);

    let default_response = handle_graph(
        State(DbState),
        Query(GraphParams {
            project: Some("proj-a".to_string()),
            include_suppressed: None,
            limit: Some(10),
        }),
    )
    .await
    .into_response();
    assert_eq!(default_response.status(), StatusCode::OK);
    let default_body = to_bytes(default_response.into_body(), usize::MAX).await?;
    let default_payload: Value = serde_json::from_slice(&default_body)?;
    assert_eq!(default_payload["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(default_payload["nodes"][0]["id"], 2);
    assert_eq!(
        default_payload["nodes"][0]["mems"],
        serde_json::json!([visible_id])
    );

    let audit_response = handle_graph(
        State(DbState),
        Query(GraphParams {
            project: Some("proj-a".to_string()),
            include_suppressed: Some(true),
            limit: Some(10),
        }),
    )
    .await
    .into_response();
    assert_eq!(audit_response.status(), StatusCode::OK);
    let audit_body = to_bytes(audit_response.into_body(), usize::MAX).await?;
    let audit_payload: Value = serde_json::from_slice(&audit_body)?;
    let node_ids: Vec<i64> = audit_payload["nodes"]
        .as_array()
        .expect("nodes should be array")
        .iter()
        .map(|node| node["id"].as_i64().expect("node id should be i64"))
        .collect();
    assert!(node_ids.contains(&1));
    assert!(node_ids.contains(&2));
    Ok(())
}

#[tokio::test]
async fn stats_excludes_expired_active_memories_and_counts_pending_review() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-stats-current");
    let conn = db::open_db()?;
    memory::insert_memory(
        &conn,
        Some("session-current"),
        "proj-a",
        None,
        "current",
        "current memory",
        "decision",
        None,
    )?;
    let expired_id = memory::insert_memory(
        &conn,
        Some("session-expired"),
        "proj-a",
        None,
        "expired",
        "expired active memory",
        "procedure",
        None,
    )?;
    conn.execute(
        "UPDATE memories SET expires_at_epoch = 1 WHERE id = ?1",
        params![expired_id],
    )?;
    conn.execute(
        "INSERT INTO memory_candidates
          (scope, memory_type, topic_key, text, evidence_event_ids, confidence,
           risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES
          ('project', 'decision', 'stats-topic', 'needs review', '[]', 0.7,
           'low', 'pending_review', 1, 1)",
        [],
    )?;
    drop(conn);

    let response = handle_stats(State(DbState)).await.into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let payload: Value = serde_json::from_slice(&body)?;
    assert_eq!(payload["active_memories"], 1);
    assert_eq!(payload["pending_candidates"], 1);
    assert_eq!(payload["type_distribution"][0]["memory_type"], "decision");
    assert_eq!(payload["type_distribution"][0]["count"], 1);
    Ok(())
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

    let (state, params, cache) = default_status_extractors();
    let response = handle_status(state, params, cache).await.into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should read");
    let payload: Value =
        serde_json::from_slice(&body).expect("status response should be valid json");
    assert_eq!(payload["memories"], stats.active_memories);
    assert_eq!(payload["observations"], stats.active_observations);
    assert_eq!(payload["captured_events"], stats.captured_events);
    assert_eq!(payload["total_observations"], stats.total_observations);
    assert_eq!(
        payload["pending_extraction_tasks"],
        stats.pending_extraction_tasks
    );
    assert_eq!(
        payload["pending_graph_candidates"],
        stats.pending_graph_candidates
    );
    assert_eq!(
        payload["promotion_funnel"]["captured_events"],
        stats.captured_events
    );
    assert_eq!(
        payload["promotion_funnel"]["observations"],
        stats.total_observations
    );
    assert_eq!(
        payload["promotion_funnel"]["candidates"],
        stats.total_memory_candidates
    );
    assert_eq!(
        payload["promotion_funnel"]["promoted"],
        stats.promoted_memory_candidates
    );
    assert_eq!(
        payload["promotion_funnel"]["pending_review"],
        stats.pending_review_memory_candidates
    );
    assert_eq!(
        payload["promotion_funnel"]["observation_rate_percent"],
        serde_json::json!(expected_percent(
            stats.total_observations,
            stats.captured_events
        ))
    );
    assert_eq!(
        payload["promotion_funnel"]["candidate_rate_percent"],
        serde_json::json!(expected_percent(
            stats.total_memory_candidates,
            stats.total_observations
        ))
    );
    assert_eq!(
        payload["promotion_funnel"]["promoted_rate_percent"],
        serde_json::json!(expected_percent(
            stats.promoted_memory_candidates,
            stats.total_memory_candidates
        ))
    );
    assert_eq!(
        payload["promotion_funnel"]["pending_review_rate_percent"],
        serde_json::json!(expected_percent(
            stats.pending_review_memory_candidates,
            stats.total_memory_candidates
        ))
    );
}

fn expected_percent(numerator: i64, denominator: i64) -> f64 {
    if denominator <= 0 {
        0.0
    } else {
        (numerator as f64 * 100.0) / denominator as f64
    }
}
