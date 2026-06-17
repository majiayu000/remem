use std::collections::HashMap;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use serde::Serialize;

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
                    observation_details_with_compressed_sources(conn, &observations).map_err(
                        |e| {
                            crate::log::warn(
                                "mcp",
                                &format!("load compressed observation sources failed: {}", e),
                            );
                            McpToolError::db_query(TOOL, e)
                        },
                    )?
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
                    let details = memory_details_with_topic_traces(
                        conn,
                        &memories,
                        params.project.as_deref(),
                    )
                    .map_err(|e| {
                        crate::log::warn("mcp", &format!("load topic traces failed: {}", e));
                        McpToolError::db_query(TOOL, e)
                    })?;
                    memory::mark_memories_accessed(conn, &params.ids).map_err(|e| {
                        crate::log::warn("mcp", &format!("mark_memories_accessed failed: {}", e));
                        McpToolError::db_query(TOOL, e)
                    })?;
                    details
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

fn observation_details_with_compressed_sources(
    conn: &rusqlite::Connection,
    observations: &[db::Observation],
) -> anyhow::Result<serde_json::Value> {
    let mut value = serde_json::to_value(observations)?;
    let Some(items) = value.as_array_mut() else {
        return Ok(value);
    };
    let observation_ids: Vec<i64> = observations
        .iter()
        .map(|observation| observation.id)
        .collect();
    let sources_by_observation = db::load_compressed_observation_sources(conn, &observation_ids)?;
    for (item, observation) in items.iter_mut().zip(observations) {
        let Some(sources) = sources_by_observation.get(&observation.id) else {
            continue;
        };
        if !sources.is_empty() {
            item["compressed_sources"] = serde_json::to_value(sources)?;
        }
    }
    Ok(value)
}

fn memory_details_with_topic_traces(
    conn: &rusqlite::Connection,
    memories: &[memory::Memory],
    requested_project: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    const TRACE_LIMIT: i64 = 12;
    let mut value = serde_json::to_value(memories)?;
    let Some(items) = value.as_array_mut() else {
        return Ok(value);
    };
    let memory_ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
    let temporal_facts = current_temporal_facts_by_memory_id(conn, &memory_ids, requested_project)?;
    let mut trace_cache = HashMap::new();
    for (item, memory) in items.iter_mut().zip(memories) {
        if let Some(facts) = temporal_facts.get(&memory.id) {
            if !facts.is_empty() {
                item["temporal_facts"] = serde_json::to_value(facts)?;
            }
        }
        let Some(topic_key) = memory.topic_key.as_deref() else {
            continue;
        };
        let trace_project = match requested_project {
            Some(project) if project == memory.project => project,
            Some(_) => continue,
            None => memory.project.as_str(),
        };
        let cache_key = (trace_project.to_string(), topic_key.to_string());
        if !trace_cache.contains_key(&cache_key) {
            let trace = db::load_trace_by_topic_key(conn, trace_project, topic_key, TRACE_LIMIT)?;
            trace_cache.insert(cache_key.clone(), trace);
        }
        let trace = trace_cache
            .get(&cache_key)
            .expect("trace cache should contain loaded key");
        if !trace.is_empty() {
            item["topic_trace"] = serde_json::to_value(trace)?;
        }
    }
    Ok(value)
}

#[derive(Serialize)]
struct MemoryTemporalFactDetail {
    project: String,
    subject: String,
    predicate: String,
    object: String,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
    learned_at_epoch: i64,
    confidence: f64,
    status: String,
}

fn current_temporal_facts_by_memory_id(
    conn: &rusqlite::Connection,
    memory_ids: &[i64],
    requested_project: Option<&str>,
) -> anyhow::Result<HashMap<i64, Vec<MemoryTemporalFactDetail>>> {
    if memory_ids.is_empty()
        || !crate::retrieval::temporal::sqlite_table_exists(conn, "memory_facts")?
    {
        return Ok(HashMap::new());
    }
    let placeholders = (1..=memory_ids.len())
        .map(|idx| format!("?{idx}"))
        .collect::<Vec<_>>()
        .join(", ");
    let has_invalidated_at_epoch = crate::memory::facts::invalidated_at_epoch_available(conn)?;
    let mut conditions = vec![
        format!("source_memory_id IN ({placeholders})"),
        crate::memory::facts::current_fact_filter_sql("", has_invalidated_at_epoch),
    ];
    let mut params = memory_ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect::<Vec<_>>();
    let now_idx = memory_ids.len() + 1;
    conditions.push(format!(
        "(valid_from_epoch IS NULL OR valid_from_epoch <= ?{now_idx})"
    ));
    conditions.push(format!(
        "(valid_to_epoch IS NULL OR valid_to_epoch > ?{now_idx})"
    ));
    params.push(Box::new(chrono::Utc::now().timestamp()));
    let mut idx = now_idx + 1;
    if let Some(project) = requested_project {
        conditions.push(format!("project = ?{idx}"));
        params.push(Box::new(project.to_string()));
        idx += 1;
    }
    let sql = format!(
        "SELECT source_memory_id, project, subject, predicate, object, valid_from_epoch,
                valid_to_epoch, learned_at_epoch, confidence, status
         FROM memory_facts
         WHERE {}
         ORDER BY source_memory_id, COALESCE(valid_from_epoch, learned_at_epoch) DESC,
                  confidence DESC, id DESC
         LIMIT ?{idx}",
        conditions.join(" AND ")
    );
    params.push(Box::new(
        (memory_ids.len() as i64).saturating_mul(12).max(12),
    ));
    let refs = crate::db::to_sql_refs(&params);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            MemoryTemporalFactDetail {
                project: row.get(1)?,
                subject: row.get(2)?,
                predicate: row.get(3)?,
                object: row.get(4)?,
                valid_from_epoch: row.get(5)?,
                valid_to_epoch: row.get(6)?,
                learned_at_epoch: row.get(7)?,
                confidence: row.get(8)?,
                status: row.get(9)?,
            },
        ))
    })?;
    let mut facts = HashMap::new();
    for row in rows {
        let (memory_id, fact) = row?;
        facts.entry(memory_id).or_insert_with(Vec::new).push(fact);
    }
    Ok(facts)
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

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rusqlite::{params, Connection};

    use super::*;

    fn memory_row(project: &str, scope: &str) -> crate::memory::Memory {
        crate::memory::Memory {
            id: 1,
            session_id: Some("session-1".to_string()),
            project: project.to_string(),
            topic_key: Some("global-contract".to_string()),
            title: "Global contract".to_string(),
            text: "Global memory body".to_string(),
            memory_type: "decision".to_string(),
            files: None,
            created_at_epoch: 10,
            updated_at_epoch: 10,
            status: "active".to_string(),
            branch: None,
            scope: scope.to_string(),
        }
    }

    fn conn_with_trace() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        conn.execute_batch("PRAGMA foreign_keys=OFF;")?;
        crate::db::insert_topic_segment(
            &conn,
            &crate::db::TopicSegmentInput {
                host_id: 1,
                project_id: 1,
                session_row_id: 1,
                project: "/repo",
                topic_key: "global-contract",
                title: "Repo-only trace",
                summary: "Trace belongs to /repo.",
                status: "resolved",
                segment_index: 0,
                covered_from_event_id: 10,
                covered_to_event_id: 12,
                evidence_event_ids: "[10,12]",
                files: None,
                confidence: 0.8,
            },
        )?;
        Ok(conn)
    }

    fn insert_memory_for_fact(conn: &Connection) -> Result<()> {
        conn.execute(
            "INSERT INTO memories
             (id, session_id, project, topic_key, title, content, memory_type, files,
              created_at_epoch, updated_at_epoch, status, branch, scope)
             VALUES (1, NULL, '/repo', 'global-contract', 'Global contract',
                     'Global memory body', 'decision', NULL, 10, 10, 'active',
                     NULL, 'project')",
            [],
        )?;
        Ok(())
    }

    #[test]
    fn topic_trace_is_omitted_for_requested_project_mismatch() -> Result<()> {
        let conn = conn_with_trace()?;
        let memories = vec![memory_row("/repo", "global")];

        let value = memory_details_with_topic_traces(&conn, &memories, Some("/other"))?;

        assert_eq!(value[0]["scope"], "global");
        assert!(value[0]["topic_trace"].is_null());
        Ok(())
    }

    #[test]
    fn topic_trace_is_attached_for_matching_requested_project() -> Result<()> {
        let conn = conn_with_trace()?;
        let memories = vec![memory_row("/repo", "project")];

        let value = memory_details_with_topic_traces(&conn, &memories, Some("/repo"))?;

        assert_eq!(value[0]["topic_trace"][0]["title"], "Repo-only trace");
        Ok(())
    }

    #[test]
    fn memory_details_attach_current_temporal_facts() -> Result<()> {
        let conn = conn_with_trace()?;
        insert_memory_for_fact(&conn)?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO memory_facts
             (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
              learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
              confidence, supersedes_fact_id, status, invalidated_at_epoch,
              created_at_epoch, updated_at_epoch)
             VALUES ('/repo', 'HarborMint', 'verified_by', 'Toma Reed', ?1, NULL, ?2, 1,
                     NULL, '[]', 0.95, NULL, 'active', NULL, ?2, ?2)",
            params![now - 1_000, now - 900],
        )?;
        let memories = vec![memory_row("/repo", "project")];

        let value = memory_details_with_topic_traces(&conn, &memories, Some("/repo"))?;

        assert_eq!(value[0]["temporal_facts"][0]["subject"], "HarborMint");
        assert_eq!(value[0]["temporal_facts"][0]["object"], "Toma Reed");
        Ok(())
    }
}
