use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::{db, memory, memory_service};

use super::helpers::{error_response, memory_to_item, open_request_db};
use super::types::{
    DbState, MemoryItem, Meta, MultiHopInfo, SaveMemoryRequest, SaveMemoryResponse, SearchParams,
    SearchResponse, ShowParams,
};

pub(super) async fn handle_search(
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
                multi_hop: results.multi_hop.map(|meta| MultiHopInfo {
                    hops: meta.hops,
                    entities_discovered: meta.entities_discovered,
                }),
            })
            .into_response()
        }
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "search_failed",
            &err.to_string(),
        )
        .into_response(),
    }
}

pub(super) async fn handle_get_memory(
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
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "db_error",
            &err.to_string(),
        )
        .into_response(),
    }
}

pub(super) async fn handle_save_memory(
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
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "save_failed",
            &err.to_string(),
        )
        .into_response(),
    }
}

pub(super) async fn handle_status(State(_state): State<DbState>) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };
    let stats = match db::query_system_stats(&conn) {
        Ok(stats) => stats,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "status_failed",
                &err.to_string(),
            )
            .into_response();
        }
    };

    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "memories": stats.active_memories,
        "observations": stats.active_observations,
    }))
    .into_response()
}
