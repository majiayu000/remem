use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct SearchParams {
    #[schemars(description = "Search query (semantic search)")]
    pub query: Option<String>,
    #[schemars(description = "Max results to return (default 20)")]
    pub limit: Option<i64>,
    #[schemars(description = "Project name filter")]
    pub project: Option<String>,
    #[schemars(description = "Observation type filter")]
    pub r#type: Option<String>,
    #[schemars(description = "Result offset for pagination")]
    pub offset: Option<i64>,
    #[schemars(description = "Include stale observations (default true, stale ranked lower)")]
    pub include_stale: Option<bool>,
    #[schemars(
        description = "Git branch filter (e.g. 'main', 'feat/auth'). Only returns memories from this branch. Old data without branch info is always included."
    )]
    pub branch: Option<String>,
    #[schemars(
        description = "Enable multi-hop search (default false). When true, performs entity graph expansion: finds entities in first-hop results, then searches for memories mentioning those entities. Use for questions that span multiple topics/people, e.g. 'What do Melanie\\'s kids like?' or 'What events has Caroline participated in?'"
    )]
    pub multi_hop: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct TimelineParams {
    #[schemars(description = "Anchor observation ID")]
    pub anchor: Option<i64>,
    #[schemars(description = "Search query to find anchor")]
    pub query: Option<String>,
    #[schemars(description = "Observations before anchor (default 5)")]
    pub depth_before: Option<i64>,
    #[schemars(description = "Observations after anchor (default 5)")]
    pub depth_after: Option<i64>,
    #[schemars(description = "Project name filter")]
    pub project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct GetObservationsParams {
    #[schemars(description = "List of observation IDs to fetch")]
    pub ids: Vec<i64>,
    #[schemars(description = "Project name filter")]
    pub project: Option<String>,
    #[schemars(description = "Source type: 'memory' or 'observation' (default: 'memory')")]
    pub source: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct SaveMemoryParams {
    #[schemars(description = "Memory text content")]
    pub text: String,
    #[schemars(description = "Optional title")]
    pub title: Option<String>,
    #[schemars(description = "Project name")]
    pub project: Option<String>,
    #[schemars(
        description = "Stable topic identifier for cross-session dedup. Same project+topic_key updates existing memory instead of creating new one. Format: kebab-case descriptive key, e.g. 'fts5-search-strategy', 'auth-middleware-design'."
    )]
    pub topic_key: Option<String>,
    #[schemars(
        description = "Memory type: decision, discovery, bugfix, architecture, preference. Defaults to 'discovery'."
    )]
    pub memory_type: Option<String>,
    #[schemars(description = "List of related file paths")]
    pub files: Option<Vec<String>>,
    #[schemars(
        description = "Optional local markdown path for backup copy. Relative paths are resolved from current working directory."
    )]
    pub local_path: Option<String>,
    #[schemars(
        description = "Memory scope: 'project' (default, only this project) or 'global' (visible in all projects). Use 'global' for user preferences and cross-project knowledge."
    )]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct TimelineReportParams {
    #[schemars(description = "Project name (required)")]
    pub project: String,
    #[schemars(description = "Full report with timeline and monthly breakdown (default false)")]
    pub full: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct WorkStreamsParams {
    #[schemars(description = "Project name filter")]
    pub project: Option<String>,
    #[schemars(description = "Status filter: active, paused, completed, abandoned")]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct UpdateWorkStreamParams {
    #[schemars(description = "WorkStream ID to update")]
    pub id: i64,
    #[schemars(description = "New status: active, paused, completed, abandoned")]
    pub status: Option<String>,
    #[schemars(description = "Next action to take")]
    pub next_action: Option<String>,
    #[schemars(description = "Current blockers")]
    pub blockers: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct SearchResult {
    pub id: i64,
    pub r#type: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    pub source: String,
    pub updated_at: String,
    pub project: String,
    pub status: String,
}
