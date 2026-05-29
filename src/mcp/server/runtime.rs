use anyhow::Result;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{tool_handler, ServerHandler, ServiceExt};

use super::MemoryServer;
use crate::db;

const SERVER_INSTRUCTIONS: &str = r#"Persistent memory for Claude Code and Codex sessions.

Retrieval:
- Use `search(query, project?)` for compact memory IDs.
- Use `get_observations(ids, source)` only for selected full details.
- Use `search_raw(query)` for literal chat recall when curated search is sparse.
- Use `timeline(anchor/query)` for chronological context around a change.

Persistence:
- Use `save_memory` only for durable decisions, bugfix root causes, important discoveries, architecture notes, or user preferences.
- For user-requested documents, write the local/project file first; memory is only a backup.
- Search before saving and use a stable kebab-case `topic_key` for repeat topics.

Workstreams:
- `workstreams(project)` lists active tasks.
- `update_workstream(id, status?, next_action?, blockers?)` updates status, next action, or blockers."#;

#[tool_handler]
impl ServerHandler for MemoryServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(SERVER_INSTRUCTIONS.into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

pub async fn run_mcp_server() -> Result<()> {
    let db_path = crate::db::db_path();
    let db_exists = db_path.exists();
    crate::log::info(
        "mcp",
        &format!(
            "server starting db={} exists={}",
            db_path.display(),
            db_exists
        ),
    );
    let server = MemoryServer::new()?;
    if let Ok(conn) = db::open_db() {
        let mem_count: i64 = conn
            .query_row("SELECT count(*) FROM memories", [], |row| row.get(0))
            .unwrap_or(-1);
        let obs_count: i64 = conn
            .query_row("SELECT count(*) FROM observations", [], |row| row.get(0))
            .unwrap_or(-1);
        crate::log::info(
            "mcp",
            &format!(
                "server ready memories={} observations={}",
                mem_count, obs_count
            ),
        );
    }
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    crate::log::info("mcp", "server stopped");
    Ok(())
}
