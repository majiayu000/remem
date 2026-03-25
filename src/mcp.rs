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
use crate::memory;
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
    #[schemars(
        description = "Git branch filter (e.g. 'main', 'feat/auth'). Only returns memories from this branch. Old data without branch info is always included."
    )]
    branch: Option<String>,
    #[schemars(
        description = "Enable multi-hop search (default false). When true, performs entity graph expansion: finds entities in first-hop results, then searches for memories mentioning those entities. Use for questions that span multiple topics/people, e.g. 'What do Melanie\\'s kids like?' or 'What events has Caroline participated in?'"
    )]
    multi_hop: Option<bool>,
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
    #[schemars(description = "Source type: 'memory' or 'observation' (default: 'memory')")]
    source: Option<String>,
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
        description = "Stable topic identifier for cross-session dedup. Same project+topic_key updates existing memory instead of creating new one. Format: kebab-case descriptive key, e.g. 'fts5-search-strategy', 'auth-middleware-design'."
    )]
    topic_key: Option<String>,
    #[schemars(
        description = "Memory type: decision, discovery, bugfix, architecture, preference. Defaults to 'discovery'."
    )]
    memory_type: Option<String>,
    #[schemars(description = "List of related file paths")]
    files: Option<Vec<String>>,
    #[schemars(
        description = "Optional local markdown path for backup copy. Relative paths are resolved from current working directory."
    )]
    local_path: Option<String>,
    #[schemars(
        description = "Memory scope: 'project' (default, only this project) or 'global' (visible in all projects). Use 'global' for user preferences and cross-project knowledge."
    )]
    scope: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TimelineReportParams {
    #[schemars(description = "Project name (required)")]
    project: String,
    #[schemars(description = "Full report with timeline and monthly breakdown (default false)")]
    full: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WorkStreamsParams {
    #[schemars(description = "Project name filter")]
    project: Option<String>,
    #[schemars(description = "Status filter: active, paused, completed, abandoned")]
    status: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UpdateWorkStreamParams {
    #[schemars(description = "WorkStream ID to update")]
    id: i64,
    #[schemars(description = "New status: active, paused, completed, abandoned")]
    status: Option<String>,
    #[schemars(description = "Next action to take")]
    next_action: Option<String>,
    #[schemars(description = "Current blockers")]
    blockers: Option<String>,
}

#[derive(Debug, Serialize)]
struct SearchResult {
    id: i64,
    r#type: String,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    topic_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview: Option<String>,
    source: String,
    updated_at: String,
    project: String,
    status: String,
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
    db::data_dir()
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
    /// Search memories by keyword/project/type.
    #[tool(
        description = "Search past memories by keyword/project/type. Returns compact results (id, type, title, topic_key, 300-char preview). WORKFLOW: search → find relevant IDs → get_observations(ids) for full details. Use when: user asks about past work, you need implementation context, or debugging a previously-fixed issue. Set multi_hop=true for questions spanning multiple people/topics (e.g. 'What do X's kids like?')."
    )]
    fn search(&self, Parameters(params): Parameters<SearchParams>) -> Result<String, String> {
        let start = std::time::Instant::now();
        // Auto-enable multi_hop when query mentions multiple entities
        let auto_multi_hop = params.query.as_deref().map_or(false, |q| {
            crate::entity::extract_entities(q, "").len() >= 2
        });
        let multi_hop = params.multi_hop.unwrap_or(auto_multi_hop);
        crate::log::info(
            "mcp",
            &format!(
                "search called query={:?} project={:?} type={:?} branch={:?} multi_hop={} limit={} offset={}",
                params.query,
                params.project,
                params.r#type,
                params.branch,
                multi_hop,
                params.limit.unwrap_or(20),
                params.offset.unwrap_or(0),
            ),
        );
        self.with_conn(|conn| {
            let limit = params.limit.unwrap_or(20);
            let (results, hop_meta) = if multi_hop {
                if let Some(q) = params.query.as_deref().filter(|q| !q.is_empty()) {
                    let mh = crate::search_multihop::search_multi_hop(
                        conn,
                        q,
                        params.project.as_deref(),
                        limit,
                    )
                    .map_err(|e| {
                        crate::log::warn("mcp", &format!("multi_hop search failed: {}", e));
                        e.to_string()
                    })?;
                    let meta = Some((mh.hops, mh.entities_discovered));
                    (mh.memories, meta)
                } else {
                    (vec![], None)
                }
            } else {
                let r = search::search_with_branch(
                    conn,
                    params.query.as_deref(),
                    params.project.as_deref(),
                    params.r#type.as_deref(),
                    limit,
                    params.offset.unwrap_or(0),
                    params.include_stale.unwrap_or(true),
                    params.branch.as_deref(),
                )
                .map_err(|e| {
                    crate::log::warn("mcp", &format!("search failed: {}", e));
                    e.to_string()
                })?;
                (r, None)
            };

            let search_results: Vec<SearchResult> = results
                .into_iter()
                .map(|m| {
                    let updated = chrono::DateTime::from_timestamp(m.updated_at_epoch, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_default();
                    let preview = m.text.chars().take(300).collect::<String>();
                    SearchResult {
                        id: m.id,
                        r#type: m.memory_type,
                        title: m.title,
                        topic_key: m.topic_key,
                        preview: Some(preview),
                        source: "memory".to_string(),
                        updated_at: updated,
                        project: m.project,
                        status: m.status,
                    }
                })
                .collect();

            let hop_info = if let Some((hops, entities)) = &hop_meta {
                format!(" hops={} entities_discovered={}", hops, entities.len())
            } else {
                String::new()
            };
            crate::log::info(
                "mcp",
                &format!(
                    "search done count={} {}ms{}",
                    search_results.len(),
                    start.elapsed().as_millis(),
                    hop_info,
                ),
            );

            // For multi-hop, include metadata about discovered entities
            if let Some((hops, entities)) = hop_meta {
                let response = serde_json::json!({
                    "results": search_results,
                    "multi_hop": {
                        "hops": hops,
                        "entities_discovered": entities,
                    }
                });
                serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
            } else {
                serde_json::to_string_pretty(&search_results).map_err(|e| e.to_string())
            }
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
        description = "Fetch complete observation details (narrative, facts, concepts, files_read, files_modified) by IDs. Use after search() to get full context. This is the second step in the search → get_observations workflow. Supports both 'memory' and 'observation' sources."
    )]
    fn get_observations(
        &self,
        Parameters(params): Parameters<GetObservationsParams>,
    ) -> Result<String, String> {
        let start = std::time::Instant::now();
        let source = params.source.as_deref().unwrap_or("memory");
        crate::log::info(
            "mcp",
            &format!(
                "get_observations called ids={:?} project={:?} source={}",
                params.ids, params.project, source
            ),
        );
        self.with_conn(|conn| {
            let results = if source == "observation" {
                // Query observations table
                let obs = db::get_observations_by_ids(conn, &params.ids, params.project.as_deref())
                    .map_err(|e| {
                        crate::log::warn("mcp", &format!("get_observations failed: {}", e));
                        e.to_string()
                    })?;
                let accessed_ids: Vec<i64> = obs.iter().map(|o| o.id).collect();
                if !accessed_ids.is_empty() {
                    if let Err(e) = db::update_last_accessed(conn, &accessed_ids) {
                        crate::log::warn("mcp", &format!("update_last_accessed failed: {}", e));
                    }
                }
                serde_json::to_value(&obs).map_err(|e| e.to_string())?
            } else {
                // Query memories table
                let memories =
                    memory::get_memories_by_ids(conn, &params.ids, params.project.as_deref())
                        .map_err(|e| {
                            crate::log::warn("mcp", &format!("get_memories failed: {}", e));
                            e.to_string()
                        })?;
                serde_json::to_value(&memories).map_err(|e| e.to_string())?
            };
            crate::log::info(
                "mcp",
                &format!(
                    "get_observations done source={} count={} {}ms",
                    source,
                    params.ids.len(),
                    start.elapsed().as_millis()
                ),
            );
            serde_json::to_string_pretty(&results).map_err(|e| e.to_string())
        })
    }

    /// Save a structured memory for future sessions
    #[tool(
        description = "Save a memory for future sessions. MUST be called after: \
        (1) architecture decisions — record what was chosen, why, and what was rejected, \
        (2) bug fixes with root cause — record symptom, root cause, fix, and prevention, \
        (3) important discoveries — record finding and its implications, \
        (4) user preferences — record preference and reasoning. \
        Use topic_key for cross-session dedup (same project+topic_key updates existing memory). \
        By default also writes a local markdown backup."
    )]
    fn save_memory(
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
        let project = params.project.as_deref().unwrap_or("manual");
        let title = params.title.as_deref().unwrap_or("Memory");
        let memory_type = params.memory_type.as_deref().unwrap_or("discovery");
        let files_json = params
            .files
            .as_ref()
            .and_then(|f| serde_json::to_string(f).ok());

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
            // Auto-detect scope: preference type defaults to global, others to project
            let scope = params
                .scope
                .as_deref()
                .unwrap_or(if memory_type == "preference" {
                    "global"
                } else {
                    "project"
                });
            let id = memory::insert_memory_full(
                conn,
                None,
                project,
                params.topic_key.as_deref(),
                title,
                &params.text,
                memory_type,
                files_json.as_deref(),
                None,
                scope,
                None,
            )
            .map_err(|e| {
                crate::log::warn("mcp", &format!("save_memory failed: {}", e));
                e.to_string()
            })?;

            let upserted = params.topic_key.is_some();
            crate::log::info(
                "mcp",
                &format!(
                    "save_memory done id={} type={} upserted={} local_status={} local_path={:?}",
                    id, memory_type, upserted, local_status, local_path_str
                ),
            );
            serde_json::to_string(&json!({
                "id": id,
                "status": "saved",
                "memory_type": memory_type,
                "upserted": upserted,
                "local_status": local_status,
                "local_path": local_path_str,
            }))
            .map_err(|e| e.to_string())
        })
    }

    /// List workstreams for a project
    #[tool(
        description = "Generate a project timeline report with activity history, type distribution, and Token ROI analysis. Use for understanding project evolution and memory system value."
    )]
    fn timeline_report(
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

    #[tool(
        description = "List active workstreams (high-level tasks tracked across sessions). Filter by project and/or status. Shows progress, next action, and blockers for each workstream."
    )]
    fn workstreams(
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
                // No project filter — return empty hint
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

    /// Manually update a workstream's status, next action, or blockers
    #[tool(
        description = "Update a workstream's status, next_action, or blockers. Use to manually mark a workstream as completed/paused/abandoned, or to update progress notes."
    )]
    fn update_workstream(
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
                 ## When to save memory (MUST follow)\n\
                 Call `save_memory` immediately when:\n\
                 1. **Making a technical decision** → type=decision, record what was chosen, why, what was rejected\n\
                 2. **Fixing a bug** → type=bugfix, record root cause, fix, how to prevent\n\
                 3. **Discovering a code constraint/pattern** → type=discovery, record finding and impact\n\
                 4. **Completing a feature module** → type=architecture, record design points and file structure\n\
                 5. **Learning a user preference** → type=preference, record preference and reasoning\n\n\
                 ## topic_key rules\n\
                 - Same topic MUST use a stable topic_key — cross-session updates to same memory instead of duplicates\n\
                 - Format: kebab-case descriptive key, e.g. \"fts5-search-strategy\", \"auth-middleware-design\"\n\
                 - Before saving, search first to check if a memory on this topic already exists\n\n\
                 ## Do NOT save\n\
                 - Single file edits (git tracks these)\n\
                 - Temporary debugging steps (only save conclusions)\n\
                 - Content that duplicates an existing memory (search first)\n\n\
                 ## Tips\n\
                 - The context index is usually sufficient — only fetch details when needed\n\
                 - bugfix and decision types often contain critical context worth fetching\n\
                 - Search supports project filter to scope results\n\
                 - Observations with status=\"stale\" may be outdated. Prefer active observations when available.\n\n\
                 ## WorkStreams\n\
                 - `workstreams(project)` lists active high-level tasks tracked across sessions\n\
                 - `update_workstream(id, status?, next_action?, blockers?)` manually updates a workstream\n\
                 - WorkStreams are auto-created from session summaries — no manual creation needed"
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
    // Quick sanity check: count memories + observations
    if let Ok(conn) = server.conn.lock() {
        let mem_count: i64 = conn
            .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
            .unwrap_or(-1);
        let obs_count: i64 = conn
            .query_row("SELECT count(*) FROM observations", [], |r| r.get(0))
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
