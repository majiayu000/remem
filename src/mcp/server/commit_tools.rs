use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};

use super::super::types::{CommitLookupParams, SessionCommitsParams};
use super::MemoryServer;

#[tool_router(router = tool_router_commit, vis = "pub(super)")]
impl MemoryServer {
    #[tool(
        description = "Look up git commit metadata and linked memory sessions by full or short SHA. The response separates git metadata from memory-derived summaries so missing links are not inferred."
    )]
    pub(super) fn lookup_commit(
        &self,
        Parameters(params): Parameters<CommitLookupParams>,
    ) -> Result<String, String> {
        crate::log::info(
            "mcp",
            &format!(
                "lookup_commit called sha={} project={:?}",
                params.sha, params.project
            ),
        );
        self.with_conn(|conn| {
            let results =
                crate::git_trace::lookup_commit(conn, params.project.as_deref(), &params.sha)
                    .map_err(|err| {
                        crate::log::warn("mcp", &format!("lookup_commit failed: {}", err));
                        err.to_string()
                    })?;
            serde_json::to_string_pretty(&results).map_err(|err| err.to_string())
        })
    }

    #[tool(
        description = "List git commits linked to a content session ID or remem memory session ID. Returns link evidence plus git metadata; it does not guess commit intent when no link exists."
    )]
    pub(super) fn commits_for_session(
        &self,
        Parameters(params): Parameters<SessionCommitsParams>,
    ) -> Result<String, String> {
        crate::log::info(
            "mcp",
            &format!(
                "commits_for_session called session_id={} project={:?} limit={}",
                params.session_id,
                params.project,
                params.limit.unwrap_or(20)
            ),
        );
        self.with_conn(|conn| {
            let results = crate::git_trace::commits_for_session(
                conn,
                params.project.as_deref(),
                &params.session_id,
                params.limit.unwrap_or(20),
            )
            .map_err(|err| {
                crate::log::warn("mcp", &format!("commits_for_session failed: {}", err));
                err.to_string()
            })?;
            serde_json::to_string_pretty(&results).map_err(|err| err.to_string())
        })
    }
}
