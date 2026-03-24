use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tower_http::cors::{Any, CorsLayer};

use crate::{db, memory, search};

type DbState = Arc<Mutex<rusqlite::Connection>>;

#[derive(Deserialize)]
struct SearchParams {
    query: Option<String>,
    project: Option<String>,
    #[serde(rename = "type")]
    memory_type: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Serialize)]
struct SearchResponse {
    data: Vec<MemoryItem>,
    meta: Meta,
}

#[derive(Serialize)]
struct MemoryItem {
    id: i64,
    title: String,
    content: String,
    memory_type: String,
    project: String,
    scope: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    topic_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    created_at_epoch: i64,
    updated_at_epoch: i64,
}

#[derive(Serialize)]
struct Meta {
    total: usize,
    limit: i64,
    offset: i64,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: String,
    message: String,
}

#[derive(Deserialize)]
struct SaveMemoryRequest {
    project: String,
    title: String,
    content: String,
    memory_type: String,
    #[serde(default)]
    topic_key: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Serialize)]
struct SaveMemoryResponse {
    id: i64,
    status: String,
}

#[derive(Deserialize)]
struct ShowParams {
    id: i64,
}

fn memory_to_item(m: &memory::Memory) -> MemoryItem {
    MemoryItem {
        id: m.id,
        title: m.title.clone(),
        content: m.text.clone(),
        memory_type: m.memory_type.clone(),
        project: m.project.clone(),
        scope: m.scope.clone(),
        status: m.status.clone(),
        topic_key: m.topic_key.clone(),
        branch: m.branch.clone(),
        created_at_epoch: m.created_at_epoch,
        updated_at_epoch: m.updated_at_epoch,
    }
}

fn error_response(status: StatusCode, code: &str, message: &str) -> impl IntoResponse {
    (
        status,
        Json(ErrorResponse {
            error: ErrorDetail {
                code: code.to_string(),
                message: message.to_string(),
            },
        }),
    )
}

async fn handle_search(
    State(db): State<DbState>,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(20).min(100);
    let offset = params.offset.unwrap_or(0);

    let conn = db.lock().unwrap();
    match search::search(
        &conn,
        params.query.as_deref(),
        params.project.as_deref(),
        params.memory_type.as_deref(),
        limit,
        offset,
        false,
    ) {
        Ok(results) => {
            let items: Vec<MemoryItem> = results.iter().map(memory_to_item).collect();
            let total = items.len();
            Json(SearchResponse {
                data: items,
                meta: Meta {
                    total,
                    limit,
                    offset,
                },
            })
            .into_response()
        }
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "search_failed",
            &e.to_string(),
        )
        .into_response(),
    }
}

async fn handle_get_memory(
    State(db): State<DbState>,
    Query(params): Query<ShowParams>,
) -> impl IntoResponse {
    let conn = db.lock().unwrap();
    match memory::get_memories_by_ids(&conn, &[params.id], None) {
        Ok(results) if !results.is_empty() => {
            Json(memory_to_item(&results[0])).into_response()
        }
        Ok(_) => error_response(StatusCode::NOT_FOUND, "not_found", "Memory not found")
            .into_response(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "db_error",
            &e.to_string(),
        )
        .into_response(),
    }
}

async fn handle_save_memory(
    State(db): State<DbState>,
    Json(req): Json<SaveMemoryRequest>,
) -> impl IntoResponse {
    let scope = req.scope.as_deref().unwrap_or("project");
    let conn = db.lock().unwrap();
    match memory::insert_memory_full(
        &conn,
        None,
        &req.project,
        req.topic_key.as_deref(),
        &req.title,
        &req.content,
        &req.memory_type,
        None,
        None,
        scope,
    ) {
        Ok(id) => (
            StatusCode::CREATED,
            Json(SaveMemoryResponse {
                id,
                status: "created".into(),
            }),
        )
            .into_response(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "save_failed",
            &e.to_string(),
        )
        .into_response(),
    }
}

async fn handle_status(State(db): State<DbState>) -> impl IntoResponse {
    let conn = db.lock().unwrap();
    let memory_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE status = 'active'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let observation_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM observations WHERE status = 'active'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "memories": memory_count,
        "observations": observation_count,
    }))
}

pub fn build_router() -> Router<DbState> {
    Router::new()
        .route("/api/v1/search", get(handle_search))
        .route("/api/v1/memory", get(handle_get_memory))
        .route("/api/v1/memories", post(handle_save_memory))
        .route("/api/v1/status", get(handle_status))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
}

pub async fn run_api_server(port: u16) -> anyhow::Result<()> {
    let conn = db::open_db()?;
    let state: DbState = Arc::new(Mutex::new(conn));

    let app = build_router().with_state(state);
    let addr = format!("127.0.0.1:{}", port);

    crate::log::info("api", &format!("REST API listening on http://{}", addr));
    println!("remem REST API v{} on http://{}", env!("CARGO_PKG_VERSION"), addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
