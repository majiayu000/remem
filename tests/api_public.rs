use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use axum::{
    body::{to_bytes, Body},
    http::{header, Method, Request, StatusCode},
};
use rusqlite::Connection;
use serde_json::{json, Value};
use tower::ServiceExt;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct TempDataDir {
    _guard: MutexGuard<'static, ()>,
    previous: Option<OsString>,
    previous_plaintext: Option<OsString>,
    path: PathBuf,
}

impl TempDataDir {
    fn new(label: &str) -> Self {
        let guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("REMEM_DATA_DIR");
        let path = std::env::temp_dir().join(format!(
            "remem-integration-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::env::set_var("REMEM_DATA_DIR", &path);
        let previous_plaintext = std::env::var_os("REMEM_ALLOW_PLAINTEXT_DB");
        std::env::set_var("REMEM_ALLOW_PLAINTEXT_DB", "1");
        Self {
            _guard: guard,
            previous,
            previous_plaintext,
            path,
        }
    }

    fn db_path(&self) -> PathBuf {
        self.path.join("remem.db")
    }
}

impl Drop for TempDataDir {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var("REMEM_DATA_DIR", previous);
        } else {
            std::env::remove_var("REMEM_DATA_DIR");
        }
        if let Some(previous) = self.previous_plaintext.as_ref() {
            std::env::set_var("REMEM_ALLOW_PLAINTEXT_DB", previous);
        } else {
            std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn authorized_request(method: Method, uri: &str, token: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(body)
        .expect("request should build")
}

fn json_request(method: Method, uri: &str, token: &str, payload: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload.to_string()))
        .expect("request should build")
}

#[test]
fn public_api_token_helpers_prepare_router_auth() {
    let _data_dir = TempDataDir::new("public-api-token");

    let token_path = remem::api::ensure_api_token().expect("token setup should succeed");
    let token = remem::api::load_api_token().expect("token should load");

    assert!(token_path.ends_with(".api-token"));
    assert_eq!(token.len(), 64);
}

#[tokio::test]
async fn exported_router_covers_auth_save_list_search_and_detail() -> anyhow::Result<()> {
    let data_dir = TempDataDir::new("public-api-router");
    remem::api::ensure_api_token()?;
    let token = remem::api::load_api_token()?;
    let app = remem::api::build_router(0).with_state(remem::api::DbState);

    let unauthorized = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/v1/status")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let status = app
        .clone()
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/status",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(status.status(), StatusCode::OK);
    let status_body = to_bytes(status.into_body(), usize::MAX).await?;
    let status_payload: Value = serde_json::from_slice(&status_body)?;
    assert_eq!(status_payload["cache"]["hit"], false);
    assert_eq!(status_payload["cache"]["stale"], false);

    let health = app
        .clone()
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/health",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(health.status(), StatusCode::OK);
    let health_body = to_bytes(health.into_body(), usize::MAX).await?;
    let health_payload: Value = serde_json::from_slice(&health_body)?;
    assert_eq!(health_payload["ok"], true);
    assert_eq!(health_payload["api_version"], 1);
    assert!(health_payload.get("token").is_none());

    let invalid_save = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/memories",
            &token,
            json!({
                "text": "   ",
                "memory_type": "decision",
                "scope": "project"
            }),
        ))
        .await?;
    assert_eq!(invalid_save.status(), StatusCode::BAD_REQUEST);
    let invalid_body = to_bytes(invalid_save.into_body(), usize::MAX).await?;
    let invalid_payload: Value = serde_json::from_slice(&invalid_body)?;
    assert_eq!(invalid_payload["error"]["code"], "save_validation_failed");

    let save = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/memories",
            &token,
            json!({
                "title": "Public API router contract",
                "text": "Router integration memory for public API contract coverage",
                "project": "public-api-project",
                "memory_type": "decision",
                "topic_key": "public-api-router-contract",
                "local_copy_enabled": false,
                "claim_enabled": false
            }),
        ))
        .await?;
    assert_eq!(save.status(), StatusCode::CREATED);
    let save_body = to_bytes(save.into_body(), usize::MAX).await?;
    let saved: Value = serde_json::from_slice(&save_body)?;
    let memory_id = saved["id"].as_i64().expect("saved memory id");
    assert_eq!(saved["project"], "public-api-project");
    assert_eq!(saved["memory_type"], "decision");
    assert_eq!(saved["operation"], "add");

    let list = app
        .clone()
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/memories?project=public-api-project",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(list.status(), StatusCode::OK);
    let list_body = to_bytes(list.into_body(), usize::MAX).await?;
    let listed: Value = serde_json::from_slice(&list_body)?;
    assert_eq!(listed["meta"]["total"], 1);
    assert_eq!(listed["data"][0]["id"], memory_id);

    let search = app
        .clone()
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/search?query=router%20integration&project=public-api-project",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(search.status(), StatusCode::OK);
    let search_body = to_bytes(search.into_body(), usize::MAX).await?;
    let searched: Value = serde_json::from_slice(&search_body)?;
    let search_hits = searched["data"].as_array().expect("search data array");
    assert!(
        search_hits.iter().any(|item| item["id"] == memory_id),
        "search response should include saved memory: {searched}"
    );

    let detail = app
        .clone()
        .oneshot(authorized_request(
            Method::GET,
            &format!("/api/v1/memories/{memory_id}"),
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(detail.status(), StatusCode::OK);
    let detail_body = to_bytes(detail.into_body(), usize::MAX).await?;
    let detailed: Value = serde_json::from_slice(&detail_body)?;
    assert_eq!(detailed["id"], memory_id);
    assert!(detailed["entities"].is_array());
    assert!(detailed["edges"].is_array());

    let conn = Connection::open(data_dir.db_path())?;
    let (access_count, last_accessed_epoch): (i64, Option<i64>) = conn.query_row(
        "SELECT access_count, last_accessed_epoch FROM memories WHERE id = ?1",
        [memory_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(access_count, 1);
    assert!(last_accessed_epoch.is_some());

    Ok(())
}
