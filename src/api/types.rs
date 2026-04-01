use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default)]
pub struct DbState;

#[derive(Deserialize)]
pub(super) struct SearchParams {
    pub query: Option<String>,
    pub project: Option<String>,
    #[serde(rename = "type")]
    pub memory_type: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub include_stale: Option<bool>,
    pub branch: Option<String>,
    pub multi_hop: Option<bool>,
}

#[derive(Serialize)]
pub(super) struct SearchResponse {
    pub data: Vec<MemoryItem>,
    pub meta: Meta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_hop: Option<MultiHopInfo>,
}

#[derive(Serialize)]
pub(super) struct MultiHopInfo {
    pub hops: u8,
    pub entities_discovered: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct MemoryItem {
    pub id: i64,
    pub title: String,
    pub content: String,
    pub memory_type: String,
    pub project: String,
    pub scope: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub created_at_epoch: i64,
    pub updated_at_epoch: i64,
}

#[derive(Serialize)]
pub(super) struct Meta {
    pub count: usize,
    pub has_more: bool,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Serialize)]
pub(super) struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Serialize)]
pub(super) struct ErrorDetail {
    pub code: String,
    pub message: String,
}

#[derive(Deserialize)]
pub(super) struct SaveMemoryRequest {
    pub text: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub topic_key: Option<String>,
    #[serde(default)]
    pub memory_type: Option<String>,
    #[serde(default)]
    pub files: Option<Vec<String>>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub created_at_epoch: Option<i64>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub local_path: Option<String>,
    #[serde(default)]
    pub local_copy_enabled: Option<bool>,
}

#[derive(Serialize)]
pub(super) struct SaveMemoryResponse {
    pub id: i64,
    pub status: String,
    pub memory_type: String,
    pub upserted: bool,
    pub local_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct ShowParams {
    pub id: i64,
}
