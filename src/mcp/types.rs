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
    #[schemars(
        description = "Include retrieval scoring and visibility explanation (default false)"
    )]
    pub explain: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct CurrentStateParams {
    #[schemars(description = "Stable state key, such as a durable topic key.")]
    pub state_key: String,
    #[schemars(
        description = "Project name filter. Defaults to repo-owned state plus global user state."
    )]
    pub project: Option<String>,
    #[schemars(description = "Memory type filter, e.g. decision or preference.")]
    pub r#type: Option<String>,
    #[schemars(description = "Explicit state-key owner scope, e.g. repo or user.")]
    pub owner_scope: Option<String>,
    #[schemars(description = "Explicit state-key owner key, e.g. a repo path or user:default.")]
    pub owner_key: Option<String>,
    #[schemars(description = "Resolve the state that applied at this Unix epoch.")]
    pub as_of_epoch: Option<i64>,
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
pub(super) struct CommitLookupParams {
    #[schemars(description = "Full or short git commit SHA to look up")]
    pub sha: String,
    #[schemars(description = "Optional project filter")]
    pub project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct SessionCommitsParams {
    #[schemars(description = "Content session ID or remem memory session ID")]
    pub session_id: String,
    #[schemars(description = "Optional project filter")]
    pub project: Option<String>,
    #[schemars(description = "Max linked commits to return (default 20, max 100)")]
    pub limit: Option<i64>,
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
        description = "Optional host session id. When provided, Stop summary promotion can suppress exact duplicate candidates from the same session."
    )]
    pub session_id: Option<String>,
    #[schemars(description = "Optional host identifier, e.g. codex-cli, claude-code, api, cli.")]
    pub host: Option<String>,
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
        description = "Optional local markdown path for backup copy. Relative paths are resolved from current working directory. The resolved path must fall within the remem data directory (REMEM_DATA_DIR); paths outside that boundary are rejected."
    )]
    pub local_path: Option<String>,
    #[schemars(
        description = "Memory scope: 'project' (default, only this project) or 'global' (visible in all projects). Use 'global' only for explicitly cross-project preferences or knowledge."
    )]
    pub scope: Option<String>,
    #[schemars(
        description = "Git branch label. If omitted, the server auto-detects from the MCP process current working directory. Pass an explicit value to override (e.g. when the calling agent is not running inside the project's git checkout)."
    )]
    pub branch: Option<String>,
    #[schemars(
        description = "Optional override for the memory's creation timestamp (Unix epoch seconds). Use only for backfilling historical entries; defaults to now when omitted."
    )]
    pub created_at_epoch: Option<i64>,
    #[schemars(
        description = "Override the local markdown backup toggle. Default behavior (when omitted) is controlled by the server config."
    )]
    pub local_copy_enabled: Option<bool>,
    #[schemars(
        description = "Override the session claim toggle. Defaults to true; set false to preserve legacy save behavior without claim rows."
    )]
    pub claim_enabled: Option<bool>,
    #[schemars(
        description = "Optional claim source label. Defaults to manual_save for MCP calls."
    )]
    pub claim_source: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct GovernMemoryParams {
    #[schemars(description = "Curated memory IDs to mutate. Use dry_run=true first to preview.")]
    pub ids: Vec<i64>,
    #[schemars(description = "Project name filter. Defaults to the MCP process current project.")]
    pub project: Option<String>,
    #[schemars(description = "Governance action: delete, reject, or stale.")]
    pub action: String,
    #[schemars(description = "Explicit user-visible reason for the mutation.")]
    pub reason: Option<String>,
    #[schemars(description = "Actor initiating the mutation, e.g. user, codex, claude.")]
    pub actor: Option<String>,
    #[schemars(
        description = "Preview affected memories without writing status changes or audit events."
    )]
    pub dry_run: Option<bool>,
    #[schemars(description = "Required true for non-dry-run destructive governance mutations.")]
    pub confirm_destructive: Option<bool>,
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
    pub source_type: String,
    pub updated_at: String,
    pub project: String,
    pub status: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct SearchRawParams {
    #[schemars(
        description = "Raw FTS query across every user/assistant turn captured by the raw archive. Unlike `search`, this bypasses all curation and returns literal chat content. Use when `search` comes back empty or you need to recall an exact phrase from past conversations."
    )]
    pub query: String,
    #[schemars(description = "Project name filter")]
    pub project: Option<String>,
    #[schemars(
        description = "Git branch filter. Returns raw rows for this branch plus older rows without branch metadata."
    )]
    pub branch: Option<String>,
    #[schemars(description = "Role filter: 'user' or 'assistant' (default: both)")]
    pub role: Option<String>,
    #[schemars(description = "Max results to return (default 20)")]
    pub limit: Option<i64>,
    #[schemars(description = "Result offset for pagination")]
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(super) struct RawSearchHit {
    pub id: i64,
    pub source_type: String,
    pub session_id: String,
    pub project: String,
    pub role: String,
    pub preview: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub created_at: String,
}
