use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use serde_json::json;

use super::super::types::{UpdateWorkStreamParams, WorkStreamsParams};
use super::errors::{self, McpToolError, McpToolResult};
use super::MemoryServer;

const WORKSTREAM_STATUSES: [&str; 4] = ["active", "paused", "completed", "abandoned"];

#[tool_router(router = tool_router_workstream, vis = "pub(super)")]
impl MemoryServer {
    #[tool(
        description = "Read-only. List existing high-level workstreams for a required project, optionally filtered by status=active, paused, completed, or abandoned. Returns a JSON array with each workstream's status, progress, next action, blockers, and timestamps. Use update_workstream to mutate an existing row; this tool does not create, update, or delete workstreams. A missing project or database failure returns a tool error."
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
        description = "Mutates one existing workstream by id. At least one of status, next_action, or blockers is required; omitted fields remain unchanged. status accepts only active, paused, completed, or abandoned. Returns a JSON object with id and updated, where updated=false means no row matched. Use workstreams to list/read rows first. This tool does not create or delete workstreams; an empty update, an unknown status, or a database failure returns a tool error."
    )]
    pub(super) fn update_workstream(
        &self,
        Parameters(params): Parameters<UpdateWorkStreamParams>,
    ) -> McpToolResult<String> {
        const TOOL: &str = "update_workstream";
        if params.status.is_none() && params.next_action.is_none() && params.blockers.is_none() {
            return Err(McpToolError::invalid_request(
                TOOL,
                "at least one of status, next_action, or blockers is required",
            ));
        }
        if let Some(status) = params.status.as_deref() {
            if !WORKSTREAM_STATUSES.contains(&status) {
                return Err(McpToolError::invalid_request(
                    TOOL,
                    format!(
                        "unknown status '{status}'; expected active, paused, completed, or abandoned"
                    ),
                ));
            }
        }
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
