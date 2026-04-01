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
}

#[derive(Debug, Clone, Default)]
pub struct SaveMemoryRequest {
    pub text: String,
    pub title: Option<String>,
    pub project: Option<String>,
    pub topic_key: Option<String>,
    pub memory_type: Option<String>,
    pub files: Option<Vec<String>>,
    pub scope: Option<String>,
    pub created_at_epoch: Option<i64>,
    pub branch: Option<String>,
    pub local_path: Option<String>,
    pub local_copy_enabled: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct SaveMemoryResult {
    pub id: i64,
    pub status: String,
    pub memory_type: String,
    pub upserted: bool,
    pub local_status: String,
    pub local_path: Option<String>,
}
