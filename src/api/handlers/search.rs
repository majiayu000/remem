use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::memory_service;

use super::super::helpers::{error_response, memory_to_item, open_request_db};
use super::super::types::{
    DbState, MemoryItem, Meta, MultiHopInfo, RawHitItem, SearchParams, SearchResponse,
};

pub(in crate::api) fn search_request_from_params(
    params: SearchParams,
) -> memory_service::SearchRequest {
    let limit = params.limit.unwrap_or(20).min(100);
    let offset = params.offset.unwrap_or(0).max(0);

    memory_service::SearchRequest {
        query: params.query,
        project: params.project,
        memory_type: params.memory_type,
        limit,
        offset,
        include_stale: params
            .include_stale
            .unwrap_or_else(memory_service::default_include_stale),
        branch: params.branch,
        multi_hop: params.multi_hop.unwrap_or(false),
    }
}

pub(in crate::api) async fn handle_search(
    State(_state): State<DbState>,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };
    let req = search_request_from_params(params);
    let limit = req.limit;
    let offset = req.offset;

    const RAW_PREVIEW_CHARS: usize = 300;

    match memory_service::search_memories(&conn, &req) {
        Ok(results) => {
            let count = results.memories.len();
            let items: Vec<MemoryItem> = results.memories.iter().map(memory_to_item).collect();
            let raw_hits: Vec<RawHitItem> = results
                .raw_hits
                .into_iter()
                .map(|msg| RawHitItem {
                    id: msg.id,
                    session_id: msg.session_id,
                    project: msg.project,
                    role: msg.role,
                    preview: msg.content.chars().take(RAW_PREVIEW_CHARS).collect(),
                    source: msg.source,
                    branch: msg.branch,
                    created_at_epoch: msg.created_at_epoch,
                })
                .collect();
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
                raw_hits,
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
