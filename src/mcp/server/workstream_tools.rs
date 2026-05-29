use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use serde_json::json;

use super::super::types::{UpdateWorkStreamParams, WorkStreamsParams};
use super::errors::{self, McpToolError, McpToolResult};
use super::MemoryServer;

#[tool_router(router = tool_router_workstream, vis = "pub(super)")]
impl MemoryServer {
    #[tool(
        description = "List active workstreams (high-level tasks tracked across sessions). Filter by project and/or status. Shows progress, next action, and blockers for each workstream."
    )]
    pub(super) fn workstreams(
        &self,
        Parameters(params): Parameters<WorkStreamsParams>,
    ) -> McpToolResult<String> {
        const TOOL: &str = "workstreams";
        crate::log::info(
            "mcp",
            &format!(
                "workstreams called project={:?} status={:?}",
                params.project, params.status
            ),
        );
        self.with_conn(TOOL, |conn| {
            let project = params.project.as_deref().unwrap_or("");
            let results = if project.is_empty() {
                return Err(McpToolError::invalid_request(
                    TOOL,
                    "project parameter required",
                ));
            } else {
                crate::workstream::query_workstreams(conn, project, params.status.as_deref())
                    .map_err(|e| {
                        crate::log::warn("mcp", &format!("workstreams query failed: {}", e));
                        McpToolError::db_query(TOOL, e)
                    })?
            };
            crate::log::info("mcp", &format!("workstreams done count={}", results.len()));
            errors::to_json_pretty(TOOL, &results)
        })
    }

    #[tool(
        description = "Update a workstream's status, next_action, or blockers. Use to manually mark a workstream as completed/paused/abandoned, or to update progress notes."
    )]
    pub(super) fn update_workstream(
        &self,
        Parameters(params): Parameters<UpdateWorkStreamParams>,
    ) -> McpToolResult<String> {
        const TOOL: &str = "update_workstream";
        crate::log::info(
            "mcp",
            &format!(
                "update_workstream called id={} status={:?}",
                params.id, params.status
            ),
        );
        self.with_conn(TOOL, |conn| {
            let updated = crate::workstream::update_workstream_manual(
                conn,
                params.id,
                params.status.as_deref(),
                params.next_action.as_deref(),
                params.blockers.as_deref(),
            )
            .map_err(|e| {
                crate::log::warn("mcp", &format!("update_workstream failed: {}", e));
                McpToolError::db_query(TOOL, e)
            })?;
            crate::log::info(
                "mcp",
                &format!(
                    "update_workstream done id={} updated={}",
                    params.id, updated
                ),
            );
            errors::to_json_string(
                TOOL,
                &json!({
                    "id": params.id,
                    "updated": updated,
                }),
            )
        })
    }
}
