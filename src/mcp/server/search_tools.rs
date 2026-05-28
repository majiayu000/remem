use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};

use super::super::types::{RawSearchHit, SearchParams, SearchResult};
use super::MemoryServer;
use crate::memory::service;

const RAW_PREVIEW_CHARS: usize = 300;

#[tool_router(router = tool_router_search, vis = "pub(super)")]
impl MemoryServer {
    #[tool(
        description = "Search past memories by keyword/project/type. Always returns a compact envelope: { mode, results, next_step, pagination }. Use result IDs with next_step.source in get_observations(ids, source) to fetch full details. raw_hits are raw_archive rows, not curated memories; use search_raw for literal chat recall.\n\n**Multi-step retrieval strategy** (follow this for complex questions):\n1. **Decompose**: Break complex questions into 2-3 focused sub-queries and search each separately. E.g. 'What do Melanie's kids like?' → search('Melanie children names') + search('Melanie kids hobbies').\n2. **Iterate**: If first search returns <5 results, extract key entities/names from results and search again with those entities.\n3. **Multi-hop**: Set multi_hop=true when the question spans multiple people/topics — this triggers entity graph expansion automatically.\n\nUse when: user asks about past work, you need implementation context, or debugging a previously-fixed issue."
    )]
    pub(super) fn search(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<String, String> {
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
        self.with_conn(|conn| {
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
                e.to_string()
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
                    serde_json::to_value(&raw_hits_json).map_err(|e| e.to_string())?;
                response["raw_hits_note"] = serde_json::Value::String(
                    "raw_hits are source_type='raw_archive' chat rows, not curated memories; use search_raw for literal recall."
                        .to_string(),
                );
            }
            if has_more {
                response["has_more"] = serde_json::Value::Bool(true);
                response["next_offset"] = serde_json::Value::from(req_offset + req_limit);
            }
            serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
        })
    }
}
