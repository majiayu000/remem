use anyhow::Result;
use chrono::Utc;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{schemars, tool, tool_handler, tool_router, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::db;
use crate::search;

#[derive(Clone)]
pub struct MemoryServer {
    tool_router: ToolRouter<Self>,
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl MemoryServer {
    pub fn new() -> Result<Self> {
        let conn = db::open_db()?;
        Ok(Self {
            tool_router: Self::tool_router(),
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn with_conn<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<T, String>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|e| format!("DB lock poisoned: {}", e))?;
        f(&conn)
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
    #[schemars(
        description = "Optional local markdown path for backup copy. Relative paths are resolved from current working directory."
    )]
    local_path: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    content_session_id: Option<String>,
}

const LOCAL_SAVE_ENABLE_ENV: &str = "REMEM_SAVE_MEMORY_LOCAL_COPY";
const LOCAL_SAVE_DIR_ENV: &str = "REMEM_SAVE_MEMORY_LOCAL_DIR";

fn env_enabled(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) => {
            let lower = v.trim().to_ascii_lowercase();
            !matches!(lower.as_str(), "0" | "false" | "no" | "off")
        }
        Err(_) => default,
    }
}

fn remem_data_dir() -> PathBuf {
    std::env::var("REMEM_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".remem")
        })
}

fn sanitize_segment(raw: &str, fallback: &str, limit: usize) -> String {
    let mut out = String::with_capacity(raw.len().min(limit));
    let mut last_underscore = false;
    for ch in raw.chars() {
        let mapped = if ch.is_ascii_alphanumeric() || ch == '_' {
            ch.to_ascii_lowercase()
        } else {
            '_'
        };
        if mapped == '_' {
            if last_underscore {
                continue;
            }
            last_underscore = true;
        } else {
            last_underscore = false;
        }
        out.push(mapped);
        if out.len() >= limit {
            break;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn default_local_note_path(project: &str, title: Option<&str>) -> PathBuf {
    let base = std::env::var(LOCAL_SAVE_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| remem_data_dir().join("manual-notes"));
    let project_dir = sanitize_segment(project, "manual", 64);
    let title_slug = sanitize_segment(title.unwrap_or("memory"), "memory", 64);
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    base.join(project_dir)
        .join(format!("{}-{}.md", ts, title_slug))
}

fn resolve_local_note_path(
    project: &str,
    title: Option<&str>,
    local_path: Option<&str>,
) -> PathBuf {
    if let Some(raw) = local_path.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    }) {
        let p = PathBuf::from(raw);
        if p.is_absolute() {
            p
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(p)
        }
    } else {
        default_local_note_path(project, title)
    }
}

fn build_local_note_content(project: &str, title: &str, text: &str) -> String {
    let now = Utc::now().to_rfc3339();
    format!(
        "---\nsource: remem.save_memory\nsaved_at: {}\nproject: {}\n---\n\n# {}\n\n{}\n",
        now, project, title, text
    )
}

fn write_local_note(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "create local note directory failed {}: {}",
                parent.display(),
                e
            )
        })?;
    }
    std::fs::write(path, content)
        .map_err(|e| format!("write local note failed {}: {}", path.display(), e))
}

#[tool_router]
impl MemoryServer {
    /// Search memory index. Returns IDs with titles. Use get_observations for full details.
    #[tool(
        description = "Search past observations by keyword/project/type. Returns compact results (id, type, title, subtitle). WORKFLOW: search → find relevant IDs → get_observations(ids) for full details. Use when: user asks about past work, you need implementation context, or debugging a previously-fixed issue."
    )]
    fn search(&self, Parameters(params): Parameters<SearchParams>) -> Result<String, String> {
        let start = std::time::Instant::now();
        crate::log::info(
            "mcp",
            &format!(
                "search called query={:?} project={:?} type={:?} limit={} offset={}",
                params.query,
                params.project,
                params.r#type,
                params.limit.unwrap_or(20),
                params.offset.unwrap_or(0),
            ),
        );
        self.with_conn(|conn| {
            let results = search::search(
                conn,
                params.query.as_deref(),
                params.project.as_deref(),
                params.r#type.as_deref(),
                params.limit.unwrap_or(20),
                params.offset.unwrap_or(0),
                params.include_stale.unwrap_or(true),
            )
            .map_err(|e| {
                crate::log::warn("mcp", &format!("search failed: {}", e));
                e.to_string()
            })?;

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
                    content_session_id: o.content_session_id,
                })
                .collect();

            crate::log::info(
                "mcp",
                &format!(
                    "search done count={} {}ms",
                    search_results.len(),
                    start.elapsed().as_millis()
                ),
            );
            serde_json::to_string_pretty(&search_results).map_err(|e| e.to_string())
        })
    }

    /// Get timeline context around an observation
    #[tool(
        description = "Get chronological observations around a specific point. Useful for understanding what happened before/after a change. Provide anchor ID or search query to find the center point."
    )]
    fn timeline(&self, Parameters(params): Parameters<TimelineParams>) -> Result<String, String> {
        let start = std::time::Instant::now();
        crate::log::info(
            "mcp",
            &format!(
                "timeline called anchor={:?} query={:?} project={:?} before={} after={}",
                params.anchor,
                params.query,
                params.project,
                params.depth_before.unwrap_or(5),
                params.depth_after.unwrap_or(5),
            ),
        );
        self.with_conn(|conn| {
            let anchor_id = if let Some(id) = params.anchor {
                id
            } else if let Some(q) = &params.query {
                let results =
                    search::search(conn, Some(q), params.project.as_deref(), None, 1, 0, true)
                        .map_err(|e| {
                            crate::log::warn("mcp", &format!("timeline search failed: {}", e));
                            e.to_string()
                        })?;
                results
                    .first()
                    .map(|o| o.id)
                    .ok_or_else(|| "No results for query".to_string())?
            } else {
                return Err("anchor or query required".to_string());
            };

            let results = db::get_timeline_around(
                conn,
                anchor_id,
                params.depth_before.unwrap_or(5),
                params.depth_after.unwrap_or(5),
                params.project.as_deref(),
            )
            .map_err(|e| {
                crate::log::warn("mcp", &format!("timeline failed: {}", e));
                e.to_string()
            })?;

            crate::log::info(
                "mcp",
                &format!(
                    "timeline done anchor={} count={} {}ms",
                    anchor_id,
                    results.len(),
                    start.elapsed().as_millis()
                ),
            );
            serde_json::to_string_pretty(&results).map_err(|e| e.to_string())
        })
    }

    /// Get full observation details by IDs
    #[tool(
        description = "Fetch complete observation details (narrative, facts, concepts, files_read, files_modified) by IDs. Use after search() to get full context. This is the second step in the search → get_observations workflow."
    )]
    fn get_observations(
        &self,
        Parameters(params): Parameters<GetObservationsParams>,
    ) -> Result<String, String> {
        let start = std::time::Instant::now();
        crate::log::info(
            "mcp",
            &format!(
                "get_observations called ids={:?} project={:?}",
                params.ids, params.project
            ),
        );
        self.with_conn(|conn| {
            let results = db::get_observations_by_ids(conn, &params.ids, params.project.as_deref())
                .map_err(|e| {
                    crate::log::warn("mcp", &format!("get_observations failed: {}", e));
                    e.to_string()
                })?;
            let accessed_ids: Vec<i64> = results.iter().map(|o| o.id).collect();
            if !accessed_ids.is_empty() {
                if let Err(e) = db::update_last_accessed(conn, &accessed_ids) {
                    crate::log::warn("mcp", &format!("update_last_accessed failed: {}", e));
                }
            }
            crate::log::info(
                "mcp",
                &format!(
                    "get_observations done count={} {}ms",
                    results.len(),
                    start.elapsed().as_millis()
                ),
            );
            serde_json::to_string_pretty(&results).map_err(|e| e.to_string())
        })
    }

    /// Manually save a memory/observation
    #[tool(
        description = "Persist an important observation/decision for future sessions. By default this tool also writes a local markdown backup copy. If user asks to 'save a document', create/update a local file first; use save_memory as long-term memory backup."
    )]
    fn save_memory(
        &self,
        Parameters(params): Parameters<SaveMemoryParams>,
    ) -> Result<String, String> {
        crate::log::info(
            "mcp",
            &format!(
                "save_memory called title={:?} project={:?} text_len={}",
                params.title,
                params.project,
                params.text.len(),
            ),
        );
        let project = params.project.as_deref().unwrap_or("manual");
        let title = params.title.as_deref().unwrap_or("Memory");
        let local_copy_enabled = env_enabled(LOCAL_SAVE_ENABLE_ENV, true);
        let mut local_path_str: Option<String> = None;
        let local_status: &str = if local_copy_enabled {
            "saved"
        } else {
            "disabled"
        };

        if local_copy_enabled {
            let local_path = resolve_local_note_path(
                project,
                params.title.as_deref(),
                params.local_path.as_deref(),
            );
            let content = build_local_note_content(project, title, &params.text);
            write_local_note(&local_path, &content).map_err(|e| {
                crate::log::warn("mcp", &format!("save_memory local copy failed: {}", e));
                e
            })?;
            local_path_str = Some(local_path.display().to_string());
        }

        self.with_conn(|conn| {
            let id = db::insert_observation(
                conn,
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
            .map_err(|e| {
                crate::log::warn("mcp", &format!("save_memory failed: {}", e));
                e.to_string()
            })?;

            crate::log::info(
                "mcp",
                &format!(
                    "save_memory done id={} local_status={} local_path={:?}",
                    id, local_status, local_path_str
                ),
            );
            serde_json::to_string(&json!({
                "id": id,
                "status": "saved",
                "local_status": local_status,
                "local_path": local_path_str,
            }))
            .map_err(|e| e.to_string())
        })
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
                 5. Use `save_memory(text)` to persist important decisions or discoveries (and local markdown backup)\n\n\
                 ## Local document rule\n\
                 - If user asks to save/write/update a document, create or edit a local file first\n\
                 - `save_memory` is long-term memory backup, not a replacement for project docs\n\n\
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
    // Quick sanity check: count observations
    {
        let conn = server.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM observations", [], |r| r.get(0))
            .unwrap_or(-1);
        let fts_count: i64 = conn
            .query_row("SELECT count(*) FROM observations_fts", [], |r| r.get(0))
            .unwrap_or(-1);
        crate::log::info(
            "mcp",
            &format!(
                "server ready observations={} fts_index={}",
                count, fts_count
            ),
        );
    }
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    crate::log::info("mcp", "server stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{resolve_local_note_path, sanitize_segment};

    #[test]
    fn sanitize_segment_collapses_invalid_chars() {
        let got = sanitize_segment("Harness / PR#33 -- Review Loop", "fallback", 64);
        assert_eq!(got, "harness_pr_33_review_loop");
    }

    #[test]
    fn resolve_relative_path_from_cwd() {
        let got = resolve_local_note_path("manual", Some("x"), Some("docs/test.md"));
        assert!(got.is_absolute());
        assert!(got.ends_with("docs/test.md"));
    }
}
