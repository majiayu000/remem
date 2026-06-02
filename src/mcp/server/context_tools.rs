use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};

use super::super::types::{GetObservationsParams, TimelineParams};
use super::errors::{self, McpToolError, McpToolResult};
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
    ) -> McpToolResult<String> {
        const TOOL: &str = "timeline";
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
        self.with_conn(TOOL, |conn| {
            let anchor_id = if let Some(id) = params.anchor {
                let anchor = db::get_observations_by_ids(conn, &[id], params.project.as_deref())
                    .map_err(|e| {
                        crate::log::warn("mcp", &format!("timeline anchor lookup failed: {}", e));
                        McpToolError::db_query(TOOL, e)
                    })?;
                if anchor.is_empty() {
                    return Err(McpToolError::not_found(
                        TOOL,
                        format!("No observation found for anchor id {id}"),
                    ));
                }
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
                    McpToolError::db_query(TOOL, e)
                })?;
                results
                    .first()
                    .map(|observation| observation.id)
                    .ok_or_else(|| {
                        McpToolError::not_found(TOOL, format!("No results for query '{query}'"))
                    })?
            } else {
                return Err(McpToolError::invalid_request(
                    TOOL,
                    "anchor or query required",
                ));
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
                McpToolError::db_query(TOOL, e)
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
            errors::to_json_pretty(TOOL, &results)
        })
    }

    #[tool(
        description = "Fetch complete details by IDs. Use after search(): pass selected IDs and the exact source from search.next_step.source or each result.source. Supports source='memory' for curated memories and source='observation' for legacy observations."
    )]
    pub(super) fn get_observations(
        &self,
        Parameters(params): Parameters<GetObservationsParams>,
    ) -> McpToolResult<String> {
        const TOOL: &str = "get_observations";
        let start = std::time::Instant::now();
        let source = params.source.as_deref().unwrap_or("memory");
        crate::log::info(
            "mcp",
            &format!(
                "get_observations called ids={:?} project={:?} source={}",
                params.ids, params.project, source
            ),
        );
        self.with_conn(TOOL, |conn| {
            let results = match source {
                "observation" => {
                    let observations_result =
                        db::get_observations_by_ids(conn, &params.ids, params.project.as_deref());
                    let observations = observations_result.map_err(|e| {
                        crate::log::warn("mcp", &format!("get_observations failed: {}", e));
                        McpToolError::db_query(TOOL, e)
                    })?;
                    ensure_requested_ids_found(
                        TOOL,
                        source,
                        &params.ids,
                        observations.iter().map(|observation| observation.id),
                    )?;
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
                    errors::to_json_value(TOOL, &observations)?
                }
                "memory" => {
                    let memories_result =
                        memory::get_memories_by_ids(conn, &params.ids, params.project.as_deref());
                    let memories = memories_result.map_err(|e| {
                        crate::log::warn("mcp", &format!("get_memories failed: {}", e));
                        McpToolError::db_query(TOOL, e)
                    })?;
                    ensure_requested_ids_found(
                        TOOL,
                        source,
                        &params.ids,
                        memories.iter().map(|memory| memory.id),
                    )?;
                    memory_details_with_topic_traces(conn, &memories).map_err(|e| {
                        crate::log::warn("mcp", &format!("load topic traces failed: {}", e));
                        McpToolError::db_query(TOOL, e)
                    })?
                }
                other => {
                    return Err(McpToolError::unsupported_source(
                        TOOL,
                        format!("unsupported source '{other}'; expected 'memory' or 'observation'"),
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
            errors::to_json_pretty(TOOL, &results)
        })
    }
}

fn memory_details_with_topic_traces(
    conn: &rusqlite::Connection,
    memories: &[memory::Memory],
) -> anyhow::Result<serde_json::Value> {
    const TRACE_LIMIT: i64 = 12;
    let mut value = serde_json::to_value(memories)?;
    let Some(items) = value.as_array_mut() else {
        return Ok(value);
    };
    for (item, memory) in items.iter_mut().zip(memories) {
        let Some(topic_key) = memory.topic_key.as_deref() else {
            continue;
        };
        let trace = db::load_trace_by_topic_key(conn, &memory.project, topic_key, TRACE_LIMIT)?;
        if !trace.is_empty() {
            item["topic_trace"] = serde_json::to_value(trace)?;
        }
    }
    Ok(value)
}

fn ensure_requested_ids_found(
    tool: &'static str,
    source: &str,
    requested_ids: &[i64],
    found_ids: impl Iterator<Item = i64>,
) -> McpToolResult<()> {
    if requested_ids.is_empty() {
        return Ok(());
    }

    let found: std::collections::HashSet<i64> = found_ids.collect();
    let missing: Vec<i64> = requested_ids
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|id| !found.contains(id))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }

    Err(McpToolError::not_found(
        tool,
        format!("{source} id(s) not found: {missing:?}"),
    ))
}
