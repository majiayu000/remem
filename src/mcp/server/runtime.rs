use anyhow::Result;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{tool_handler, ServerHandler, ServiceExt};

use super::MemoryServer;
use crate::db;

const SERVER_INSTRUCTIONS: &str = r#"Persistent memory for Claude Code sessions.

## Workflow
1. **Context index** is auto-injected at session start (titles + types, ~50 tokens each)
2. When you need details: `search(query)` → get matching IDs
3. Then: `get_observations(ids)` → full narrative, facts, concepts, files
4. Use `timeline(anchor/query)` to understand chronological context around a change
5. Use `save_memory(text)` to persist important decisions or discoveries (and local markdown backup)

## Local document rule
- If user asks to save/write/update a document, create or edit a local file first
- `save_memory` is long-term memory backup, not a replacement for project docs

## When to search
- User asks about past work, previous sessions, or "what did we do"
- You need implementation details for code you're about to modify
- Debugging an issue that may have been fixed before
- Looking for architecture decisions or rationale

## Search strategy for complex questions
- **Decompose** complex questions into 2-3 focused sub-queries and call search() for each
- **Iterate**: if <5 results, extract names/entities from results and search again
- **Multi-hop**: set multi_hop=true when spanning multiple people or topics

## When to save memory (MUST follow)
Call `save_memory` immediately when:
1. **Making a technical decision** → type=decision, record what was chosen, why, what was rejected
2. **Fixing a bug** → type=bugfix, record root cause, fix, how to prevent
3. **Discovering a code constraint/pattern** → type=discovery, record finding and impact
4. **Completing a feature module** → type=architecture, record design points and file structure
5. **Learning a user preference** → type=preference, record preference and reasoning

## topic_key rules
- Same topic MUST use a stable topic_key — cross-session updates to same memory instead of duplicates
- Format: kebab-case descriptive key, e.g. "fts5-search-strategy", "auth-middleware-design"
- Before saving, search first to check if a memory on this topic already exists

## Do NOT save
- Single file edits (git tracks these)
- Temporary debugging steps (only save conclusions)
- Content that duplicates an existing memory (search first)

## Tips
- The context index is usually sufficient — only fetch details when needed
- bugfix and decision types often contain critical context worth fetching
- Search supports project filter to scope results
- Observations with status="stale" may be outdated. Prefer active observations when available.

## WorkStreams
- `workstreams(project)` lists active high-level tasks tracked across sessions
- `update_workstream(id, status?, next_action?, blockers?)` manually updates a workstream
- WorkStreams are auto-created from session summaries — no manual creation needed"#;

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
