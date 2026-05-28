use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};

use super::super::types::{GetObservationsParams, TimelineParams};
use super::MemoryServer;
use crate::retrieval::search;
use crate::{db, memory};

#[tool_router(router = tool_router_context, vis = "pub(super)")]
impl MemoryServer {
    #[tool(
        description = "Get chronological observations around a specific point. Useful for understanding what happened before/after a change. Provide anchor ID or search query to find the center point."
    )]
    pub(super) fn timeline(
        &self,
        Parameters(params): Parameters<TimelineParams>,
    ) -> Result<String, String> {
        let start = std::time::Instant::now();
        crate::log::info(
            "mcp",
            &format!(
                "timeline called anchor={:?} query={:?} project={:?} before={} after={}",
                params.anchor,
                params.query,
                params.project,
                params.depth_before.unwrap_or(5),
                params.depth_after.unwrap_or(5),
            ),
        );
        self.with_conn(|conn| {
            let anchor_id = if let Some(id) = params.anchor {
                id
            } else if let Some(query) = &params.query {
                let results = search::search(
                    conn,
                    Some(query),
                    params.project.as_deref(),
                    None,
                    1,
                    0,
                    true,
                )
                .map_err(|e| {
                    crate::log::warn("mcp", &format!("timeline search failed: {}", e));
                    e.to_string()
                })?;
                results
                    .first()
                    .map(|observation| observation.id)
                    .ok_or_else(|| "No results for query".to_string())?
            } else {
                return Err("anchor or query required".to_string());
            };

            let results = db::get_timeline_around(
                conn,
                anchor_id,
                params.depth_before.unwrap_or(5),
                params.depth_after.unwrap_or(5),
                params.project.as_deref(),
            )
            .map_err(|e| {
                crate::log::warn("mcp", &format!("timeline failed: {}", e));
                e.to_string()
            })?;

            crate::log::info(
                "mcp",
                &format!(
                    "timeline done anchor={} count={} {}ms",
                    anchor_id,
                    results.len(),
                    start.elapsed().as_millis()
                ),
            );
            serde_json::to_string_pretty(&results).map_err(|e| e.to_string())
        })
    }

    #[tool(
        description = "Fetch complete details by IDs. Use after search(): pass selected IDs and the exact source from search.next_step.source or each result.source. Supports source='memory' for curated memories and source='observation' for legacy observations."
    )]
    pub(super) fn get_observations(
        &self,
        Parameters(params): Parameters<GetObservationsParams>,
    ) -> Result<String, String> {
        let start = std::time::Instant::now();
        let source = params.source.as_deref().unwrap_or("memory");
        crate::log::info(
            "mcp",
            &format!(
                "get_observations called ids={:?} project={:?} source={}",
                params.ids, params.project, source
            ),
        );
        self.with_conn(|conn| {
            let results = match source {
                "observation" => {
                    let observations_result =
                        db::get_observations_by_ids(conn, &params.ids, params.project.as_deref());
                    let observations = observations_result.map_err(|e| {
                        crate::log::warn("mcp", &format!("get_observations failed: {}", e));
                        e.to_string()
                    })?;
                    let accessed_ids: Vec<i64> = observations
                        .iter()
                        .map(|observation| observation.id)
                        .collect();
                    if !accessed_ids.is_empty() {
                        if let Err(err) = db::update_last_accessed(conn, &accessed_ids) {
                            crate::log::warn(
                                "mcp",
                                &format!("update_last_accessed failed: {}", err),
                            );
                        }
                    }
                    serde_json::to_value(&observations).map_err(|e| e.to_string())?
                }
                "memory" => {
                    let memories_result =
                        memory::get_memories_by_ids(conn, &params.ids, params.project.as_deref());
                    let memories = memories_result.map_err(|e| {
                        crate::log::warn("mcp", &format!("get_memories failed: {}", e));
                        e.to_string()
                    })?;
                    serde_json::to_value(&memories).map_err(|e| e.to_string())?
                }
                other => {
                    return Err(format!(
                        "unsupported source '{other}'; expected 'memory' or 'observation'"
                    ));
                }
            };
            crate::log::info(
                "mcp",
                &format!(
                    "get_observations done source={} count={} {}ms",
                    source,
                    params.ids.len(),
                    start.elapsed().as_millis()
                ),
            );
            serde_json::to_string_pretty(&results).map_err(|e| e.to_string())
        })
    }
}
