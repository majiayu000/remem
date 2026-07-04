use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};

use super::super::types::{ListRawSessionsParams, RawSearchHit, SearchRawParams};
use super::errors::{self, McpToolError, McpToolResult};
use super::MemoryServer;
use crate::memory::raw_archive;

const PREVIEW_CHARS: usize = 300;

#[tool_router(router = tool_router_raw, vis = "pub(super)")]
impl MemoryServer {
    #[tool(
        description = "Search the raw archive (every user/assistant turn captured by the Stop hook). \
        Use this when `search` returns no curated match or you need to recall a literal phrase from past chats. \
        Returns the untreated conversation content — expect noise. \
        The raw archive is what guarantees 'what was said remains searchable' even when summarize/promote skip a turn."
    )]
    pub(super) fn search_raw(
        &self,
        Parameters(params): Parameters<SearchRawParams>,
    ) -> McpToolResult<String> {
        const TOOL: &str = "search_raw";
        let start = std::time::Instant::now();
        crate::log::info(
            "mcp",
            &format!(
                "search_raw called query={:?} project={:?} branch={:?} role={:?} limit={} offset={}",
                params.query,
                params.project,
                params.branch,
                params.role,
                params.limit.unwrap_or(20),
                params.offset.unwrap_or(0),
            ),
        );
        let since_epoch = parse_optional_bound(TOOL, "since", params.since.as_deref())?;
        let until_epoch = parse_optional_bound(TOOL, "until", params.until.as_deref())?;
        self.with_conn(TOOL, |conn| {
            let req = raw_archive::RawSearchRequest {
                query: params.query.clone(),
                project: params.project.clone(),
                branch: params.branch.clone(),
                role: params.role.clone(),
                limit: params.limit.unwrap_or(20),
                offset: params.offset.unwrap_or(0),
                since_epoch,
                until_epoch,
            };
            let hits = raw_archive::search_raw_messages(conn, &req).map_err(|e| {
                crate::log::warn("mcp", &format!("search_raw failed: {}", e));
                McpToolError::db_query(TOOL, e)
            })?;

            let results: Vec<RawSearchHit> = hits
                .into_iter()
                .map(|msg| RawSearchHit {
                    id: msg.id,
                    source_type: "raw_archive".to_string(),
                    session_id: msg.session_id,
                    project: msg.project,
                    role: msg.role,
                    preview: msg.content.chars().take(PREVIEW_CHARS).collect(),
                    source: msg.source,
                    branch: msg.branch,
                    created_at: chrono::DateTime::from_timestamp(msg.created_at_epoch, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_default(),
                })
                .collect();

            crate::log::info(
                "mcp",
                &format!(
                    "search_raw done count={} {}ms",
                    results.len(),
                    start.elapsed().as_millis()
                ),
            );
            errors::to_json_pretty(TOOL, &results)
        })
    }

    #[tool(
        description = "List sessions with raw archive messages inside a time window, grouped by \
        (source_root, project, session_id) with first/last message epoch, message count, and optional \
        role=user message samples. Use for recap-style summaries of what happened in a period. \
        Output fields match `remem raw sessions --json`."
    )]
    pub(super) fn list_raw_sessions(
        &self,
        Parameters(params): Parameters<ListRawSessionsParams>,
    ) -> McpToolResult<String> {
        const TOOL: &str = "list_raw_sessions";
        let start = std::time::Instant::now();
        crate::log::info(
            "mcp",
            &format!(
                "list_raw_sessions called since={:?} until={:?} project={:?} sample={}",
                params.since,
                params.until,
                params.project,
                params.sample.unwrap_or(0),
            ),
        );
        let since_epoch = parse_optional_bound(TOOL, "since", params.since.as_deref())?;
        let until_epoch = parse_optional_bound(TOOL, "until", params.until.as_deref())?;
        self.with_conn(TOOL, |conn| {
            let query = raw_archive::RawSessionQuery {
                since_epoch,
                until_epoch,
                project: params.project.clone(),
                sample_user_messages: params.sample.unwrap_or(0).max(0),
            };
            let sessions = raw_archive::list_sessions(conn, &query).map_err(|e| {
                crate::log::warn("mcp", &format!("list_raw_sessions failed: {}", e));
                McpToolError::db_query(TOOL, e)
            })?;

            crate::log::info(
                "mcp",
                &format!(
                    "list_raw_sessions done count={} {}ms",
                    sessions.len(),
                    start.elapsed().as_millis()
                ),
            );
            errors::to_json_pretty(TOOL, &raw_archive::build_sessions_json(&query, sessions))
        })
    }
}

fn parse_optional_bound(
    tool: &'static str,
    field: &str,
    value: Option<&str>,
) -> Result<Option<i64>, McpToolError> {
    value
        .map(raw_archive::parse_time_bound)
        .transpose()
        .map_err(|e| {
            crate::log::warn("mcp", &format!("{tool} invalid {field}: {e}"));
            McpToolError::invalid_request(tool, e.to_string())
        })
}
