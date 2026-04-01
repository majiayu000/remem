use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};

use super::super::types::{SearchParams, SearchResult};
use super::MemoryServer;
use crate::memory_service;

#[tool_router(router = tool_router_search, vis = "pub(super)")]
impl MemoryServer {
    #[tool(
        description = "Search past memories by keyword/project/type. Returns compact results (id, type, title, topic_key, 300-char preview). WORKFLOW: search → find relevant IDs → get_observations(ids) for full details.\n\n**Multi-step retrieval strategy** (follow this for complex questions):\n1. **Decompose**: Break complex questions into 2-3 focused sub-queries and search each separately. E.g. 'What do Melanie's kids like?' → search('Melanie children names') + search('Melanie kids hobbies').\n2. **Iterate**: If first search returns <5 results, extract key entities/names from results and search again with those entities.\n3. **Multi-hop**: Set multi_hop=true when the question spans multiple people/topics — this triggers entity graph expansion automatically.\n\nUse when: user asks about past work, you need implementation context, or debugging a previously-fixed issue."
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
            let req = memory_service::SearchRequest {
                query: params.query.clone(),
                project: params.project.clone(),
                memory_type: params.r#type.clone(),
                limit: params.limit.unwrap_or(20),
                offset: params.offset.unwrap_or(0),
                include_stale: params.include_stale.unwrap_or(true),
                branch: params.branch.clone(),
                multi_hop: requested_multi_hop,
            };
            let search_set = memory_service::search_memories(conn, &req).map_err(|e| {
                crate::log::warn("mcp", &format!("search failed: {}", e));
                e.to_string()
            })?;
            let memory_service::SearchResultSet {
                memories,
                multi_hop,
                has_more: _,
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
                        updated_at: updated,
                        project: memory.project,
                        status: memory.status,
                    }
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
                    "search done count={} {}ms{}",
                    search_results.len(),
                    start.elapsed().as_millis(),
                    hop_info,
                ),
            );

            if let Some(meta) = multi_hop {
                let response = serde_json::json!({
                    "results": search_results,
                    "multi_hop": {
                        "hops": meta.hops,
                        "entities_discovered": meta.entities_discovered,
                    }
                });
                serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
            } else {
                serde_json::to_string_pretty(&search_results).map_err(|e| e.to_string())
            }
        })
    }
}
