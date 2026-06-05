use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};

use super::super::types::{CurrentStateParams, RawSearchHit, SearchParams, SearchResult};
use super::errors::{self, McpToolError, McpToolResult};
use super::MemoryServer;
use crate::memory::service;

const RAW_PREVIEW_CHARS: usize = 300;

#[tool_router(router = tool_router_search, vis = "pub(super)")]
impl MemoryServer {
    #[tool(
        description = "Resolve the current memory/fact state for a stable state key. Returns explicit current, not_found, ambiguous, or unresolved_conflict status plus compact history and why edges."
    )]
    pub(super) fn current_state(
        &self,
        Parameters(params): Parameters<CurrentStateParams>,
    ) -> McpToolResult<String> {
        const TOOL: &str = "current_state";
        if params.state_key.trim().is_empty() {
            return Err(McpToolError::invalid_request(TOOL, "state_key is required"));
        }
        self.with_conn(TOOL, |conn| {
            let req = service::CurrentStateRequest {
                state_key: params.state_key.clone(),
                project: params.project.clone(),
                owner_scope: params.owner_scope.clone(),
                owner_key: params.owner_key.clone(),
                memory_type: params.r#type.clone(),
                as_of_epoch: params.as_of_epoch,
                include_history: true,
            };
            let result = service::current_state(conn, &req).map_err(|e| {
                crate::log::warn("mcp", &format!("current_state failed: {}", e));
                McpToolError::db_query(TOOL, e)
            })?;
            errors::to_json_pretty(TOOL, &result)
        })
    }

    #[tool(
        description = "Search curated memories by query/project/type. Returns compact results with IDs, source='memory', pagination, and next_step for get_observations(ids, source). Use search_raw for literal chat recall."
    )]
    pub(super) fn search(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> McpToolResult<String> {
        const TOOL: &str = "search";
        let start = std::time::Instant::now();
        let requested_multi_hop = params.multi_hop.unwrap_or(false);
        crate::log::info(
            "mcp",
            &format!(
                "search called query={:?} project={:?} type={:?} branch={:?} multi_hop={} limit={} offset={}",
                params.query,
                params.project,
                params.r#type,
                params.branch,
                requested_multi_hop,
                params.limit.unwrap_or(20),
                params.offset.unwrap_or(0),
            ),
        );
        self.with_conn(TOOL, |conn| {
            let req = service::SearchRequest {
                query: params.query.clone(),
                project: params.project.clone(),
                memory_type: params.r#type.clone(),
                limit: params.limit.unwrap_or(20),
                offset: params.offset.unwrap_or(0),
                include_stale: params
                    .include_stale
                    .unwrap_or_else(service::default_include_stale),
                branch: params.branch.clone(),
                multi_hop: requested_multi_hop,
                explain: false,
            };
            let search_set = service::search_memories(conn, &req).map_err(|e| {
                crate::log::warn("mcp", &format!("search failed: {}", e));
                McpToolError::db_query(TOOL, e)
            })?;
            let req_limit = req.limit;
            let req_offset = req.offset;
            let service::SearchResultSet {
                memories,
                multi_hop,
                has_more,
                explain: _,
                raw_hits,
            } = search_set;

            let search_results: Vec<SearchResult> = memories
                .into_iter()
                .map(|memory| {
                    let updated = chrono::DateTime::from_timestamp(memory.updated_at_epoch, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_default();
                    let preview = memory.text.chars().take(300).collect::<String>();
                    SearchResult {
                        id: memory.id,
                        r#type: memory.memory_type,
                        title: memory.title,
                        topic_key: memory.topic_key,
                        preview: Some(preview),
                        source: "memory".to_string(),
                        source_type: "memory".to_string(),
                        updated_at: updated,
                        project: memory.project,
                        status: memory.status,
                    }
                })
                .collect();

            let raw_hits_json: Vec<RawSearchHit> = raw_hits
                .into_iter()
                .map(|msg| RawSearchHit {
                    id: msg.id,
                    source_type: "raw_archive".to_string(),
                    session_id: msg.session_id,
                    project: msg.project,
                    role: msg.role,
                    preview: msg.content.chars().take(RAW_PREVIEW_CHARS).collect(),
                    source: msg.source,
                    branch: msg.branch,
                    created_at: chrono::DateTime::from_timestamp(msg.created_at_epoch, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_default(),
                })
                .collect();

            let hop_info = if let Some(meta) = &multi_hop {
                format!(
                    " hops={} entities_discovered={}",
                    meta.hops,
                    meta.entities_discovered.len()
                )
            } else {
                String::new()
            };
            crate::log::info(
                "mcp",
                &format!(
                    "search done count={} raw_fallback={} {}ms{}",
                    search_results.len(),
                    raw_hits_json.len(),
                    start.elapsed().as_millis(),
                    hop_info,
                ),
            );

            let result_ids: Vec<i64> = search_results.iter().map(|result| result.id).collect();
            let next_offset = has_more.then_some(req_offset + req_limit);
            let mut response = serde_json::json!({
                "mode": "compact",
                "results": search_results,
                "next_step": {
                    "tool": "get_observations",
                    "source": "memory",
                    "ids": result_ids,
                    "reason": "Pass selected compact result IDs with source='memory' to fetch full details."
                },
                "pagination": {
                    "limit": req_limit,
                    "offset": req_offset,
                    "has_more": has_more,
                    "next_offset": next_offset,
                }
            });
            if let Some(meta) = multi_hop {
                response["multi_hop"] = serde_json::json!({
                    "hops": meta.hops,
                    "entities_discovered": meta.entities_discovered,
                });
            }
            if !raw_hits_json.is_empty() {
                response["raw_hits"] =
                    errors::to_json_value(TOOL, &raw_hits_json)?;
                response["raw_hits_note"] = serde_json::Value::String(
                    "raw_hits are source_type='raw_archive' chat rows, not curated memories; use search_raw for literal recall."
                        .to_string(),
                );
            }
            if has_more {
                response["has_more"] = serde_json::Value::Bool(true);
                response["next_offset"] = serde_json::Value::from(req_offset + req_limit);
            }
            errors::to_json_pretty(TOOL, &response)
        })
    }
}
