use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::memory::service;

use super::super::helpers::{error_response, memory_to_item, open_request_db};
use super::super::types::{
    DbState, MemoryItem, Meta, MultiHopInfo, RawHitItem, SearchParams, SearchResponse,
};

pub(in crate::api) fn search_request_from_params(params: SearchParams) -> service::SearchRequest {
    let limit = params.limit.unwrap_or(20).min(100);
    let offset = params.offset.unwrap_or(0).max(0);

    service::SearchRequest {
        query: params.query,
        project: params.project,
        memory_type: params.memory_type,
        limit,
        offset,
        include_stale: params
            .include_stale
            .unwrap_or_else(service::default_include_stale),
        branch: params.branch,
        multi_hop: params.multi_hop.unwrap_or(false),
        explain: params.explain.unwrap_or(false),
    }
}

pub(in crate::api) async fn handle_search(
    State(_state): State<DbState>,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    if params.explain.unwrap_or(false)
        && params
            .query
            .as_deref()
            .is_none_or(|query| query.trim().is_empty())
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_search_request",
            "explain requires a non-empty query; set query or explain=false",
        )
        .into_response();
    }

    if params.multi_hop.unwrap_or(false) && params.explain.unwrap_or(false) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_search_request",
            "explain is not supported with multi_hop search yet; set multi_hop=false or explain=false",
        )
        .into_response();
    }

    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };
    let req = search_request_from_params(params);
    let limit = req.limit;
    let offset = req.offset;

    const RAW_PREVIEW_CHARS: usize = 300;

    match service::search_memories(&conn, &req) {
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
                explain: results.explain,
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

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use axum::{
        body::to_bytes,
        extract::{Query, State},
        response::IntoResponse,
    };
    use serde_json::Value;

    use super::*;
    use crate::db::test_support::ScopedTestDataDir;
    use crate::memory;

    fn base_search_params(explain: Option<bool>) -> SearchParams {
        SearchParams {
            query: Some("aurora".to_string()),
            project: Some("/repo".to_string()),
            memory_type: None,
            limit: Some(5),
            offset: Some(0),
            include_stale: Some(true),
            branch: None,
            multi_hop: Some(false),
            explain,
        }
    }

    fn multi_hop_explain_params() -> SearchParams {
        SearchParams {
            multi_hop: Some(true),
            explain: Some(true),
            ..base_search_params(None)
        }
    }

    #[test]
    fn search_request_from_params_keeps_explain_default_false() {
        let request = search_request_from_params(base_search_params(None));

        assert!(!request.explain);
    }

    #[test]
    fn search_request_from_params_passes_explain_true() {
        let request = search_request_from_params(base_search_params(Some(true)));

        assert!(request.explain);
    }

    #[tokio::test]
    async fn handle_search_emits_explain_only_when_requested() -> Result<()> {
        let _dir = ScopedTestDataDir::new("api-search-explain");
        let conn = crate::db::open_db()?;
        let memory_id = memory::insert_memory(
            &conn,
            Some("session-1"),
            "/repo",
            Some("aurora-contract"),
            "Aurora contract decision",
            "The aurora recall contract keeps search compact before expansion.",
            "decision",
            None,
        )?;
        drop(conn);

        let default_response = handle_search(State(DbState), Query(base_search_params(None)))
            .await
            .into_response();
        let default_body = to_bytes(default_response.into_body(), usize::MAX).await?;
        let default_json: Value = serde_json::from_slice(&default_body)?;
        assert!(default_json.get("explain").is_none());
        assert_eq!(default_json["data"][0]["id"], memory_id);
        assert_eq!(default_json["data"][0]["staleness"]["status"], "active");
        assert_eq!(
            default_json["data"][0]["staleness"]["source_anchor"],
            "untracked"
        );

        let explain_response = handle_search(State(DbState), Query(base_search_params(Some(true))))
            .await
            .into_response();
        let explain_body = to_bytes(explain_response.into_body(), usize::MAX).await?;
        let explain_json: Value = serde_json::from_slice(&explain_body)?;

        assert_eq!(explain_json["data"][0]["id"], memory_id);
        assert_eq!(explain_json["data"][0]["staleness"]["status"], "active");
        assert_eq!(
            explain_json["data"][0]["staleness"]["source_anchor"],
            "untracked"
        );
        assert_eq!(explain_json["explain"]["query"], "aurora");
        assert_eq!(
            explain_json["explain"]["results"][0]["memory_id"],
            memory_id
        );
        assert_eq!(
            explain_json["explain"]["results"][0]["staleness"]["status"],
            "active"
        );
        Ok(())
    }

    #[tokio::test]
    async fn handle_search_rejects_multi_hop_explain() -> Result<()> {
        let _dir = ScopedTestDataDir::new("api-search-explain-multi-hop");

        let response = handle_search(State(DbState), Query(multi_hop_explain_params()))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let json: Value = serde_json::from_slice(&body)?;

        assert_eq!(json["error"]["code"], "invalid_search_request");
        assert!(
            json["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("multi_hop"),
            "{}",
            json
        );
        Ok(())
    }

    #[tokio::test]
    async fn handle_search_rejects_explain_without_query() -> Result<()> {
        for query in [None, Some("")] {
            let mut params = base_search_params(Some(true));
            params.query = query.map(str::to_string);

            let response = handle_search(State(DbState), Query(params))
                .await
                .into_response();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let body = to_bytes(response.into_body(), usize::MAX).await?;
            let json: Value = serde_json::from_slice(&body)?;

            assert_eq!(json["error"]["code"], "invalid_search_request");
            assert!(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("non-empty query"),
                "{}",
                json
            );
        }
        Ok(())
    }
}
