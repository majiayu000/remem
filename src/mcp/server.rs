mod context_tools;
mod runtime;
mod search_tools;
#[cfg(test)]
mod tests;
mod workstream_tools;
mod write_tools;

use anyhow::Result;
use rmcp::handler::server::router::tool::ToolRouter;

#[derive(Clone)]
pub(super) struct MemoryServer {
    tool_router: ToolRouter<Self>,
}

impl MemoryServer {
    fn new() -> Result<Self> {
        Ok(Self {
            tool_router: Self::tool_router_search()
                + Self::tool_router_context()
                + Self::tool_router_write()
                + Self::tool_router_workstream(),
        })
    }

    fn with_conn<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<T, String>,
    {
        let conn = crate::db::open_db().map_err(|e| format!("DB open failed: {}", e))?;
        f(&conn)
    }
}

pub use runtime::run_mcp_server;
