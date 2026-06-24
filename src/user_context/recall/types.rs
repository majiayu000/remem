use serde::Serialize;

pub(super) const DEFAULT_LIMIT: usize = 12;
pub(super) const MAX_LIMIT: usize = 50;
pub(super) const DEFAULT_BUDGET_CHARS: usize = 4_000;
pub(super) const MIN_BUDGET_CHARS: usize = 500;
pub(super) const MAX_BUDGET_CHARS: usize = 12_000;
pub(super) const MAX_CLAIM_SCAN: i64 = 200;
pub(super) const MAX_SESSION_SCAN: i64 = 50;

#[derive(Debug, Clone)]
pub struct UserRecallRequest {
    pub query: String,
    pub project: String,
    pub task_intent: Option<String>,
    pub current_files: Vec<String>,
    pub host: Option<String>,
    pub owner_scope: Option<String>,
    pub owner_key: Option<String>,
    pub state_keys: Vec<String>,
    pub include_sensitive: bool,
    pub include_suppressed: bool,
    pub limit: Option<i64>,
    pub budget_chars: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserRecallResult {
    pub query: String,
    pub project: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_intent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub empty: bool,
    pub context: String,
    pub usage_policy: Option<&'static str>,
    pub included: Vec<UserRecallItem>,
    pub dropped: Vec<UserRecallDroppedItem>,
    pub diagnostics: UserRecallDiagnostics,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserRecallItem {
    pub source_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub text: String,
    pub reason_codes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_refs: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserRecallDroppedItem {
    pub source_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub reason_code: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserRecallDiagnostics {
    pub requested_limit: usize,
    pub budget_chars: usize,
    pub used_chars: usize,
    pub candidate_counts: UserRecallCandidateCounts,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UserRecallCandidateCounts {
    pub summaries: usize,
    pub claims: usize,
    pub memories: usize,
    pub current_state: usize,
    pub workstreams: usize,
    pub sessions: usize,
    pub dropped: usize,
}

#[derive(Debug, Clone)]
pub(super) struct NormalizedRequest {
    pub(super) query: String,
    pub(super) project: String,
    pub(super) task_intent: Option<String>,
    pub(super) host: Option<String>,
    pub(super) owner_scope: String,
    pub(super) owner_key: String,
    pub(super) state_keys: Vec<String>,
    pub(super) include_sensitive: bool,
    pub(super) include_suppressed: bool,
    pub(super) limit: usize,
    pub(super) budget_chars: usize,
    pub(super) terms: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct RecallCandidate {
    pub(super) source_type: String,
    pub(super) source_id: Option<i64>,
    pub(super) title: Option<String>,
    pub(super) text: String,
    pub(super) reason_codes: Vec<String>,
    pub(super) source_refs: Option<serde_json::Value>,
    pub(super) priority: i32,
}

#[derive(Debug, Clone)]
pub(super) struct ClaimCandidate {
    pub(super) id: i64,
    pub(super) claim_type: String,
    pub(super) claim_key: String,
    pub(super) claim_text: String,
    pub(super) owner_scope: String,
    pub(super) owner_key: String,
    pub(super) sensitivity: String,
    pub(super) source_refs_json: String,
    pub(super) status: String,
    pub(super) valid_from_epoch: Option<i64>,
    pub(super) valid_to_epoch: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RecallState {
    pub(super) candidates: Vec<RecallCandidate>,
    pub(super) dropped: Vec<UserRecallDroppedItem>,
    pub(super) counts: UserRecallCandidateCounts,
}
