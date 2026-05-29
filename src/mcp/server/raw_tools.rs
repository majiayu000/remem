use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};

use super::super::types::{RawSearchHit, SearchRawParams};
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
        self.with_conn(TOOL, |conn| {
            let req = raw_archive::RawSearchRequest {
                query: params.query.clone(),
                project: params.project.clone(),
                branch: params.branch.clone(),
                role: params.role.clone(),
                limit: params.limit.unwrap_or(20),
                offset: params.offset.unwrap_or(0),
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
}
