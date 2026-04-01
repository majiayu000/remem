use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use serde_json::json;

use super::super::types::{SaveMemoryParams, TimelineReportParams};
use super::MemoryServer;
use crate::memory_service;

#[tool_router(router = tool_router_write, vis = "pub(super)")]
impl MemoryServer {
    #[tool(
        description = "Save a memory for future sessions. MUST be called after: \
        (1) architecture decisions — record what was chosen, why, and what was rejected, \
        (2) bug fixes with root cause — record symptom, root cause, fix, and prevention, \
        (3) important discoveries — record finding and its implications, \
        (4) user preferences — record preference and reasoning. \
        Use topic_key for cross-session dedup (same project+topic_key updates existing memory). \
        By default also writes a local markdown backup."
    )]
    pub(super) fn save_memory(
        &self,
        Parameters(params): Parameters<SaveMemoryParams>,
    ) -> Result<String, String> {
        crate::log::info(
            "mcp",
            &format!(
                "save_memory called title={:?} project={:?} type={:?} topic_key={:?} text_len={}",
                params.title,
                params.project,
                params.memory_type,
                params.topic_key,
                params.text.len(),
            ),
        );
        self.with_conn(|conn| {
            let req = memory_service::SaveMemoryRequest {
                text: params.text.clone(),
                title: params.title.clone(),
                project: params.project.clone(),
                topic_key: params.topic_key.clone(),
                memory_type: params.memory_type.clone(),
                files: params.files.clone(),
                scope: params.scope.clone(),
                created_at_epoch: None,
                branch: None,
                local_path: params.local_path.clone(),
                local_copy_enabled: None,
            };
            let saved = memory_service::save_memory(conn, &req).map_err(|e| {
                crate::log::warn("mcp", &format!("save_memory failed: {}", e));
                e.to_string()
            })?;

            crate::log::info(
                "mcp",
                &format!(
                    "save_memory done id={} type={} upserted={} local_status={} local_path={:?}",
                    saved.id,
                    saved.memory_type,
                    saved.upserted,
                    saved.local_status,
                    saved.local_path
                ),
            );
            serde_json::to_string(&json!({
                "id": saved.id,
                "status": saved.status,
                "memory_type": saved.memory_type,
                "upserted": saved.upserted,
                "local_status": saved.local_status,
                "local_path": saved.local_path,
            }))
            .map_err(|e| e.to_string())
        })
    }

    #[tool(
        description = "Generate a project timeline report with activity history, type distribution, and Token ROI analysis. Use for understanding project evolution and memory system value."
    )]
    pub(super) fn timeline_report(
        &self,
        Parameters(params): Parameters<TimelineReportParams>,
    ) -> Result<String, String> {
        let full = params.full.unwrap_or(false);
        crate::log::info(
            "mcp",
            &format!("timeline_report project={:?} full={}", params.project, full),
        );
        self.with_conn(|conn| {
            crate::timeline::generate_timeline_report(conn, &params.project, full)
                .map_err(|e| e.to_string())
        })
    }
}
