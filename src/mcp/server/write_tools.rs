use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use serde_json::json;

use super::super::types::{GovernMemoryParams, SaveMemoryParams, TimelineReportParams};
use super::errors::{self, McpToolError, McpToolResult};
use super::MemoryServer;
use crate::{db, memory::service};

fn detect_branch_from_cwd() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let cwd_str = cwd.to_str()?;
    db::detect_git_branch(cwd_str)
}

fn map_save_memory_error(tool: &'static str, err: anyhow::Error) -> McpToolError {
    if err.is::<service::LocalCopyError>() {
        McpToolError::invalid_request(tool, err.to_string())
    } else {
        McpToolError::db_query(tool, err)
    }
}

fn validate_governance_request(
    tool: &'static str,
    params: &GovernMemoryParams,
) -> McpToolResult<()> {
    if !params.ids.iter().any(|id| *id > 0) {
        return Err(McpToolError::invalid_request(
            tool,
            "memory governance requires at least one memory id",
        ));
    }
    if params.dry_run.unwrap_or(false) {
        return Ok(());
    }
    if !params.confirm_destructive.unwrap_or(false) {
        return Err(McpToolError::invalid_request(
            tool,
            "memory governance mutation requires confirm_destructive=true",
        ));
    }
    let has_reason = params
        .reason
        .as_deref()
        .map(str::trim)
        .is_some_and(|reason| !reason.is_empty());
    if !has_reason {
        return Err(McpToolError::invalid_request(
            tool,
            "memory governance mutation requires an explicit reason",
        ));
    }
    Ok(())
}

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
    ) -> McpToolResult<String> {
        const TOOL: &str = "save_memory";
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
        let branch = params
            .branch
            .clone()
            .filter(|b| !b.trim().is_empty())
            .or_else(detect_branch_from_cwd);
        self.with_conn(TOOL, move |conn| {
            let req = service::SaveMemoryRequest {
                text: params.text.clone(),
                title: params.title.clone(),
                project: params.project.clone(),
                session_id: params.session_id.clone(),
                host: params
                    .host
                    .clone()
                    .filter(|host| !host.trim().is_empty())
                    .or_else(|| Some("codex-cli".to_string())),
                topic_key: params.topic_key.clone(),
                memory_type: params.memory_type.clone(),
                files: params.files.clone(),
                scope: params.scope.clone(),
                created_at_epoch: params.created_at_epoch,
                branch,
                local_path: params.local_path.clone(),
                local_copy_enabled: params.local_copy_enabled,
                claim_enabled: params.claim_enabled,
                claim_source: params
                    .claim_source
                    .clone()
                    .filter(|source| !source.trim().is_empty())
                    .or_else(|| Some("manual_save".to_string())),
            };
            let saved = service::save_memory_with_reference_time(
                conn,
                &req,
                params.reference_time_epoch,
            )
            .map_err(|e| {
                crate::log::warn("mcp", &format!("save_memory failed: {}", e));
                map_save_memory_error(TOOL, e)
            })?;

            crate::log::info(
                "mcp",
                &format!(
                    "save_memory done id={} type={} operation={} upserted={} local_status={} local_path={:?}",
                    saved.id,
                    saved.memory_type,
                    saved.operation,
                    saved.upserted,
                    saved.local_status,
                    saved.local_path
                ),
            );
            errors::to_json_string(
                TOOL,
                &json!({
                    "id": saved.id,
                    "status": saved.status,
                    "memory_type": saved.memory_type,
                    "project": saved.project,
                    "scope": saved.scope,
                    "topic_key": saved.topic_key,
                    "branch": saved.branch,
                    "operation": saved.operation,
                    "created_at_epoch": saved.created_at_epoch,
                    "reference_time_epoch": saved.reference_time_epoch,
                    "updated_at_epoch": saved.updated_at_epoch,
                    "upserted": saved.upserted,
                    "local_copy": {
                        "status": saved.local_copy.status,
                        "path": saved.local_copy.path,
                        "reason": saved.local_copy.reason,
                    },
                    "local_status": saved.local_status,
                    "local_path": saved.local_path,
                    "claim_status": saved.claim_status,
                    "claim_id": saved.claim_id,
                    "claim_error": saved.claim_error,
                    "next_step": {
                        "tool": saved.next_step.tool,
                        "ids": saved.next_step.ids,
                        "source": saved.next_step.source,
                        "reason": saved.next_step.reason,
                    },
                }),
            )
        })
    }

    #[tool(
        description = "Auditably delete, reject, or mark curated memories stale. \
        Use dry_run=true first to preview affected IDs. Non-dry-run mutations require \
        confirm_destructive=true and an explicit reason. This never deletes raw archive data."
    )]
    pub(super) fn govern_memory(
        &self,
        Parameters(params): Parameters<GovernMemoryParams>,
    ) -> McpToolResult<String> {
        const TOOL: &str = "govern_memory";
        crate::log::info(
            "mcp",
            &format!(
                "govern_memory called action={} project={:?} ids={} dry_run={:?}",
                params.action,
                params.project,
                params.ids.len(),
                params.dry_run
            ),
        );
        let action = crate::memory::governance::MemoryGovernanceAction::parse(&params.action)
            .map_err(|e| McpToolError::invalid_request(TOOL, e.to_string()))?;
        validate_governance_request(TOOL, &params)?;
        let project = params
            .project
            .clone()
            .filter(|project| !project.trim().is_empty())
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|cwd| db::project_from_cwd(&cwd.to_string_lossy()))
                    .unwrap_or_else(|| "unknown".to_string())
            });
        self.with_conn(TOOL, move |conn| {
            let result = crate::memory::governance::govern_memories(
                conn,
                &crate::memory::governance::GovernMemoryRequest {
                    project: &project,
                    ids: &params.ids,
                    action,
                    reason: params.reason.as_deref(),
                    actor: params.actor.as_deref().or(Some("mcp")),
                    dry_run: params.dry_run.unwrap_or(false),
                    confirm_destructive: params.confirm_destructive.unwrap_or(false),
                },
            )
            .map_err(|e| {
                crate::log::warn("mcp", &format!("govern_memory failed: {}", e));
                McpToolError::db_query(TOOL, e)
            })?;
            errors::to_json_string(TOOL, &result)
        })
    }

    #[tool(
        description = "Generate a project timeline report with activity history, type distribution, and Token ROI analysis. Use for understanding project evolution and memory system value."
    )]
    pub(super) fn timeline_report(
        &self,
        Parameters(params): Parameters<TimelineReportParams>,
    ) -> McpToolResult<String> {
        const TOOL: &str = "timeline_report";
        let full = params.full.unwrap_or(false);
        crate::log::info(
            "mcp",
            &format!("timeline_report project={:?} full={}", params.project, full),
        );
        self.with_conn(TOOL, |conn| {
            crate::timeline::generate_timeline_report(conn, &params.project, full)
                .map_err(|e| McpToolError::db_query(TOOL, e))
        })
    }
}
