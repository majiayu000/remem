#[derive(Debug, Clone, Default)]
pub struct SearchRequest {
    pub query: Option<String>,
    pub project: Option<String>,
    pub memory_type: Option<String>,
    pub limit: i64,
    pub offset: i64,
    pub include_stale: bool,
    pub branch: Option<String>,
    pub multi_hop: bool,
    pub explain: bool,
}

/// Canonical default for `include_stale` across every adapter (MCP, REST, CLI).
///
/// Default search returns only current curated memories. Callers that need
/// stale or archived history must opt in explicitly.
pub fn default_include_stale() -> bool {
    false
}

#[derive(Debug, Clone)]
pub struct MultiHopMeta {
    pub hops: u8,
    pub entities_discovered: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SearchResultSet {
    pub memories: Vec<crate::memory::Memory>,
    pub multi_hop: Option<MultiHopMeta>,
    pub has_more: bool,
    pub explain: Option<crate::retrieval::search::SearchExplain>,
    /// Raw archive hits attached as fallback when curated memories are sparse.
    pub raw_hits: Vec<crate::memory::raw_archive::RawMessage>,
}

#[derive(Debug, Clone, Default)]
pub struct SaveMemoryRequest {
    pub text: String,
    pub title: Option<String>,
    pub project: Option<String>,
    pub session_id: Option<String>,
    pub host: Option<String>,
    pub topic_key: Option<String>,
    pub memory_type: Option<String>,
    pub files: Option<Vec<String>>,
    pub scope: Option<String>,
    pub created_at_epoch: Option<i64>,
    pub branch: Option<String>,
    pub local_path: Option<String>,
    pub local_copy_enabled: Option<bool>,
    pub claim_enabled: Option<bool>,
    pub claim_source: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LocalCopyResult {
    pub status: String,
    pub path: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SaveMemoryNextStep {
    pub tool: String,
    pub ids: Vec<i64>,
    pub source: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct SaveMemoryResult {
    pub id: i64,
    pub status: String,
    pub memory_type: String,
    pub project: String,
    pub scope: String,
    pub topic_key: Option<String>,
    pub branch: Option<String>,
    pub operation: String,
    pub created_at_epoch: i64,
    pub updated_at_epoch: i64,
    /// Compatibility alias: true when the request supplied `topic_key`.
    /// It does not mean the durable row was updated; use `operation` for that.
    pub upserted: bool,
    pub local_copy: LocalCopyResult,
    pub local_status: String,
    pub local_path: Option<String>,
    pub claim_status: String,
    pub claim_id: Option<i64>,
    pub claim_error: Option<String>,
    pub next_step: SaveMemoryNextStep,
}
