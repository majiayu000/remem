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

use crate::{db, memory, memory_service};

type DbState = Arc<Mutex<rusqlite::Connection>>;

#[derive(Deserialize)]
struct SearchParams {
    query: Option<String>,
    project: Option<String>,
    #[serde(rename = "type")]
    memory_type: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
    include_stale: Option<bool>,
    branch: Option<String>,
    multi_hop: Option<bool>,
}

#[derive(Serialize)]
struct SearchResponse {
    data: Vec<MemoryItem>,
    meta: Meta,
    #[serde(skip_serializing_if = "Option::is_none")]
    multi_hop: Option<MultiHopInfo>,
}

#[derive(Serialize)]
struct MultiHopInfo {
    hops: u8,
    entities_discovered: Vec<String>,
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
    count: usize,
    has_more: bool,
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
    text: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    topic_key: Option<String>,
    #[serde(default)]
    memory_type: Option<String>,
    #[serde(default)]
    files: Option<Vec<String>>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    created_at_epoch: Option<i64>,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    local_path: Option<String>,
    #[serde(default)]
    local_copy_enabled: Option<bool>,
}

#[derive(Serialize)]
struct SaveMemoryResponse {
    id: i64,
    status: String,
    memory_type: String,
    upserted: bool,
    local_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_path: Option<String>,
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
    let offset = params.offset.unwrap_or(0).max(0);

    let Ok(conn) = db.lock() else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "lock_failed",
            "database lock poisoned",
        )
        .into_response();
    };

    let req = memory_service::SearchRequest {
        query: params.query,
        project: params.project,
        memory_type: params.memory_type,
        limit,
        offset,
        include_stale: params.include_stale.unwrap_or(false),
        branch: params.branch,
        multi_hop: params.multi_hop.unwrap_or(false),
    };

    match memory_service::search_memories(&conn, &req) {
        Ok(results) => {
            let count = results.memories.len();
            let has_more = count as i64 >= limit;
            let items: Vec<MemoryItem> = results.memories.iter().map(memory_to_item).collect();
            Json(SearchResponse {
                data: items,
                meta: Meta {
                    count,
                    has_more,
                    limit,
                    offset,
                },
                multi_hop: results.multi_hop.map(|m| MultiHopInfo {
                    hops: m.hops,
                    entities_discovered: m.entities_discovered,
                }),
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
    let Ok(conn) = db.lock() else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "lock_failed",
            "database lock poisoned",
        )
        .into_response();
    };
    match memory::get_memories_by_ids(&conn, &[params.id], None) {
        Ok(results) if !results.is_empty() => Json(memory_to_item(&results[0])).into_response(),
        Ok(_) => {
            error_response(StatusCode::NOT_FOUND, "not_found", "Memory not found").into_response()
        }
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
    let Ok(conn) = db.lock() else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "lock_failed",
            "database lock poisoned",
        )
        .into_response();
    };

    let save_req = memory_service::SaveMemoryRequest {
        text: req.text,
        title: req.title,
        project: req.project,
        topic_key: req.topic_key,
        memory_type: req.memory_type,
        files: req.files,
        scope: req.scope,
        created_at_epoch: req.created_at_epoch,
        branch: req.branch,
        local_path: req.local_path,
        local_copy_enabled: req.local_copy_enabled,
    };

    match memory_service::save_memory(&conn, &save_req) {
        Ok(saved) => (
            StatusCode::CREATED,
            Json(SaveMemoryResponse {
                id: saved.id,
                status: saved.status,
                memory_type: saved.memory_type,
                upserted: saved.upserted,
                local_status: saved.local_status,
                local_path: saved.local_path,
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
    let Ok(conn) = db.lock() else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "lock_failed",
            "database lock poisoned",
        )
        .into_response();
    };
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
    .into_response()
}

pub fn build_router() -> Router<DbState> {
    Router::new()
        .route("/api/v1/search", get(handle_search))
        .route("/api/v1/memory", get(handle_get_memory))
        .route("/api/v1/memories", post(handle_save_memory))
        .route("/api/v1/status", get(handle_status))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
}

pub async fn run_api_server(port: u16) -> anyhow::Result<()> {
    let conn = db::open_db()?;
    let state: DbState = Arc::new(Mutex::new(conn));

    let app = build_router().with_state(state);
    let addr = format!("127.0.0.1:{}", port);

    crate::log::info("api", &format!("REST API listening on http://{}", addr));
    println!(
        "remem REST API v{} on http://{}",
        env!("CARGO_PKG_VERSION"),
        addr
    );

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
