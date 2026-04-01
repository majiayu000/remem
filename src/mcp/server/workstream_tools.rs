use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use serde_json::json;

use super::super::types::{UpdateWorkStreamParams, WorkStreamsParams};
use super::MemoryServer;

#[tool_router(router = tool_router_workstream, vis = "pub(super)")]
impl MemoryServer {
    #[tool(
        description = "List active workstreams (high-level tasks tracked across sessions). Filter by project and/or status. Shows progress, next action, and blockers for each workstream."
    )]
    pub(super) fn workstreams(
        &self,
        Parameters(params): Parameters<WorkStreamsParams>,
    ) -> Result<String, String> {
        crate::log::info(
            "mcp",
            &format!(
                "workstreams called project={:?} status={:?}",
                params.project, params.status
            ),
        );
        self.with_conn(|conn| {
            let project = params.project.as_deref().unwrap_or("");
            let results = if project.is_empty() {
                return Ok(r#"{"error": "project parameter required"}"#.to_string());
            } else {
                crate::workstream::query_workstreams(conn, project, params.status.as_deref())
                    .map_err(|e| {
                        crate::log::warn("mcp", &format!("workstreams query failed: {}", e));
                        e.to_string()
                    })?
            };
            crate::log::info("mcp", &format!("workstreams done count={}", results.len()));
            serde_json::to_string_pretty(&results).map_err(|e| e.to_string())
        })
    }

    #[tool(
        description = "Update a workstream's status, next_action, or blockers. Use to manually mark a workstream as completed/paused/abandoned, or to update progress notes."
    )]
    pub(super) fn update_workstream(
        &self,
        Parameters(params): Parameters<UpdateWorkStreamParams>,
    ) -> Result<String, String> {
        crate::log::info(
            "mcp",
            &format!(
                "update_workstream called id={} status={:?}",
                params.id, params.status
            ),
        );
        self.with_conn(|conn| {
            let updated = crate::workstream::update_workstream_manual(
                conn,
                params.id,
                params.status.as_deref(),
                params.next_action.as_deref(),
                params.blockers.as_deref(),
            )
            .map_err(|e| {
                crate::log::warn("mcp", &format!("update_workstream failed: {}", e));
                e.to_string()
            })?;
            crate::log::info(
                "mcp",
                &format!(
                    "update_workstream done id={} updated={}",
                    params.id, updated
                ),
            );
            serde_json::to_string(&json!({
                "id": params.id,
                "updated": updated,
            }))
            .map_err(|e| e.to_string())
        })
    }
}
