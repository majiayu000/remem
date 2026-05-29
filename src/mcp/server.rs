mod commit_tools;
mod context_tools;
mod errors;
mod raw_tools;
mod runtime;
mod search_tools;
#[cfg(test)]
mod tests;
mod workstream_tools;
mod write_tools;

use anyhow::Result;
use rmcp::handler::server::router::tool::ToolRouter;

use errors::{McpToolError, McpToolResult};

#[derive(Clone)]
pub(super) struct MemoryServer {
    tool_router: ToolRouter<Self>,
}

impl MemoryServer {
    fn new() -> Result<Self> {
        Ok(Self {
            tool_router: Self::tool_router_search()
                + Self::tool_router_context()
                + Self::tool_router_commit()
                + Self::tool_router_write()
                + Self::tool_router_workstream()
                + Self::tool_router_raw(),
        })
    }

    fn with_conn<F, T>(&self, tool: &'static str, f: F) -> McpToolResult<T>
    where
        F: FnOnce(&rusqlite::Connection) -> McpToolResult<T>,
    {
        let conn = crate::db::open_db().map_err(|e| McpToolError::db_open(tool, e))?;
        f(&conn)
    }
}

pub use runtime::run_mcp_server;
