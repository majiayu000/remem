use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};

use crate::{db, memory, memory_service};

#[derive(Clone, Copy, Default)]
pub struct DbState;

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

fn open_request_db() -> Result<rusqlite::Connection, Response> {
    db::open_db().map_err(|e| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "db_open_failed",
            &e.to_string(),
        )
        .into_response()
    })
}

async fn handle_search(
    State(_state): State<DbState>,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(20).min(100);
    let offset = params.offset.unwrap_or(0).max(0);
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
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
            let items: Vec<MemoryItem> = results.memories.iter().map(memory_to_item).collect();
            Json(SearchResponse {
                data: items,
                meta: Meta {
                    count,
                    has_more: results.has_more,
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
    State(_state): State<DbState>,
    Query(params): Query<ShowParams>,
) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
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
    State(_state): State<DbState>,
    Json(req): Json<SaveMemoryRequest>,
) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
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

async fn handle_status(State(_state): State<DbState>) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };
    let stats = match db::query_system_stats(&conn) {
        Ok(stats) => stats,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "status_failed",
                &e.to_string(),
            )
            .into_response()
        }
    };

    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "memories": stats.active_memories,
        "observations": stats.active_observations,
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
    let app = build_router().with_state(DbState);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::ScopedTestDataDir;
    use crate::{db, memory};
    use axum::body::to_bytes;
    use rusqlite::params;
    use serde_json::Value;

    #[test]
    fn db_state_is_stateless() {
        assert_eq!(std::mem::size_of::<DbState>(), 0);
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
    }
}
