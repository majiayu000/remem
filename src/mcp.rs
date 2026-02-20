use anyhow::Result;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::db;
use crate::search;

#[derive(Clone)]
pub struct MemoryServer {
    tool_router: ToolRouter<Self>,
}

impl MemoryServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchParams {
    #[schemars(description = "Search query (semantic search)")]
    query: Option<String>,
    #[schemars(description = "Max results to return (default 20)")]
    limit: Option<i64>,
    #[schemars(description = "Project name filter")]
    project: Option<String>,
    #[schemars(description = "Observation type filter")]
    r#type: Option<String>,
    #[schemars(description = "Result offset for pagination")]
    offset: Option<i64>,
    #[schemars(description = "Include stale observations (default true, stale ranked lower)")]
    include_stale: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TimelineParams {
    #[schemars(description = "Anchor observation ID")]
    anchor: Option<i64>,
    #[schemars(description = "Search query to find anchor")]
    query: Option<String>,
    #[schemars(description = "Observations before anchor (default 5)")]
    depth_before: Option<i64>,
    #[schemars(description = "Observations after anchor (default 5)")]
    depth_after: Option<i64>,
    #[schemars(description = "Project name filter")]
    project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetObservationsParams {
    #[schemars(description = "List of observation IDs to fetch")]
    ids: Vec<i64>,
    #[schemars(description = "Project name filter")]
    project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SaveMemoryParams {
    #[schemars(description = "Memory text content")]
    text: String,
    #[schemars(description = "Optional title")]
    title: Option<String>,
    #[schemars(description = "Project name")]
    project: Option<String>,
}

#[derive(Debug, Serialize)]
struct SearchResult {
    id: i64,
    r#type: String,
    title: Option<String>,
    subtitle: Option<String>,
    created_at: String,
    project: Option<String>,
    status: String,
}

#[tool_router]
impl MemoryServer {
    /// Search memory index. Returns IDs with titles. Use get_observations for full details.
    #[tool(description = "Search past observations by keyword/project/type. Returns compact results (id, type, title, subtitle). WORKFLOW: search → find relevant IDs → get_observations(ids) for full details. Use when: user asks about past work, you need implementation context, or debugging a previously-fixed issue.")]
    fn search(&self, Parameters(params): Parameters<SearchParams>) -> Result<String, String> {
        let conn = db::open_db().map_err(|e| e.to_string())?;
        let results = search::search(
            &conn,
            params.query.as_deref(),
            params.project.as_deref(),
            params.r#type.as_deref(),
            params.limit.unwrap_or(20),
            params.offset.unwrap_or(0),
            params.include_stale.unwrap_or(true),
        )
        .map_err(|e| e.to_string())?;

        let search_results: Vec<SearchResult> = results
            .into_iter()
            .map(|o| SearchResult {
                id: o.id,
                r#type: o.r#type,
                title: o.title,
                subtitle: o.subtitle,
                created_at: o.created_at,
                project: o.project,
                status: o.status,
            })
            .collect();

        serde_json::to_string_pretty(&search_results).map_err(|e| e.to_string())
    }

    /// Get timeline context around an observation
    #[tool(description = "Get chronological observations around a specific point. Useful for understanding what happened before/after a change. Provide anchor ID or search query to find the center point.")]
    fn timeline(&self, Parameters(params): Parameters<TimelineParams>) -> Result<String, String> {
        let conn = db::open_db().map_err(|e| e.to_string())?;

        let anchor_id = if let Some(id) = params.anchor {
            id
        } else if let Some(q) = &params.query {
            let results = search::search(&conn, Some(q), params.project.as_deref(), None, 1, 0, true)
                .map_err(|e| e.to_string())?;
            results
                .first()
                .map(|o| o.id)
                .ok_or_else(|| "No results for query".to_string())?
        } else {
            return Err("anchor or query required".to_string());
        };

        let results = db::get_timeline_around(
            &conn,
            anchor_id,
            params.depth_before.unwrap_or(5),
            params.depth_after.unwrap_or(5),
            params.project.as_deref(),
        )
        .map_err(|e| e.to_string())?;

        serde_json::to_string_pretty(&results).map_err(|e| e.to_string())
    }

    /// Get full observation details by IDs
    #[tool(description = "Fetch complete observation details (narrative, facts, concepts, files_read, files_modified) by IDs. Use after search() to get full context. This is the second step in the search → get_observations workflow.")]
    fn get_observations(&self, Parameters(params): Parameters<GetObservationsParams>) -> Result<String, String> {
        let conn = db::open_db().map_err(|e| e.to_string())?;
        let results = db::get_observations_by_ids(&conn, &params.ids).map_err(|e| e.to_string())?;
        if !params.ids.is_empty() {
            let _ = db::update_last_accessed(&conn, &params.ids);
        }
        serde_json::to_string_pretty(&results).map_err(|e| e.to_string())
    }

    /// Manually save a memory/observation
    #[tool(description = "Manually save an important observation, decision, or learning to persistent memory. Use when you discover something worth remembering for future sessions (architecture decisions, gotchas, user preferences).")]
    fn save_memory(&self, Parameters(params): Parameters<SaveMemoryParams>) -> Result<String, String> {
        let conn = db::open_db().map_err(|e| e.to_string())?;
        let project = params.project.as_deref().unwrap_or("manual");

        let id = db::insert_observation(
            &conn,
            "manual",
            project,
            "discovery",
            params.title.as_deref(),
            None,
            Some(&params.text),
            None,
            None,
            None,
            None,
            None,
            0,
        )
        .map_err(|e| e.to_string())?;

        Ok(format!("{{\"id\": {}, \"status\": \"saved\"}}", id))
    }
}

#[tool_handler]
impl ServerHandler for MemoryServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Persistent memory for Claude Code sessions.\n\n\
                 ## Workflow\n\
                 1. **Context index** is auto-injected at session start (titles + types, ~50 tokens each)\n\
                 2. When you need details: `search(query)` → get matching IDs\n\
                 3. Then: `get_observations(ids)` → full narrative, facts, concepts, files\n\
                 4. Use `timeline(anchor/query)` to understand chronological context around a change\n\
                 5. Use `save_memory(text)` to persist important decisions or discoveries\n\n\
                 ## When to search\n\
                 - User asks about past work, previous sessions, or \"what did we do\"\n\
                 - You need implementation details for code you're about to modify\n\
                 - Debugging an issue that may have been fixed before\n\
                 - Looking for architecture decisions or rationale\n\n\
                 ## Tips\n\
                 - The context index is usually sufficient — only fetch details when needed\n\
                 - bugfix and decision types often contain critical context worth fetching\n\
                 - Search supports project filter to scope results\n\
                 - Observations with status=\"stale\" may be outdated. Prefer active observations when available."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

pub async fn run_mcp_server() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let server = MemoryServer::new();
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
