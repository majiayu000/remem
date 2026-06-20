use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct SummaryRequest<'a> {
    pub owner_scope: Option<&'a str>,
    pub owner_key: Option<&'a str>,
    pub project: &'a str,
}

#[derive(Debug, Clone)]
pub struct SummaryEditRequest<'a> {
    pub owner_scope: Option<&'a str>,
    pub owner_key: Option<&'a str>,
    pub project: &'a str,
    pub text: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserContextSummary {
    pub id: i64,
    pub user_key: String,
    pub owner_scope: String,
    pub owner_key: String,
    pub scope: String,
    pub scope_key: Option<String>,
    pub summary_text: String,
    pub source_claim_ids: Vec<i64>,
    pub source_memory_ids: Vec<i64>,
    pub source_activity_refs: Vec<ActivityRef>,
    pub status: String,
    pub model: Option<String>,
    pub version: i64,
    pub created_at_epoch: i64,
    pub updated_at_epoch: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SummarySources {
    pub summary: Option<UserContextSummary>,
    pub included_claims: Vec<SummaryClaimSource>,
    pub included_memories: Vec<SummaryMemorySource>,
    pub included_activity_refs: Vec<ActivityRef>,
    pub dropped_claims: Vec<DroppedSource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SummaryClaimSource {
    pub id: i64,
    pub claim_type: String,
    pub claim_key: String,
    pub claim_text: String,
    pub owner_scope: String,
    pub owner_key: String,
    pub sensitivity: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SummaryMemorySource {
    pub id: i64,
    pub memory_type: String,
    pub title: String,
    pub preview: String,
    pub owner_scope: Option<String>,
    pub owner_key: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivityRef {
    pub kind: String,
    pub id: i64,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DroppedSource {
    pub kind: String,
    pub id: i64,
    pub reason: String,
}

pub(super) struct SourceBundle {
    pub(super) claims: Vec<SummaryClaimSource>,
    pub(super) memories: Vec<SummaryMemorySource>,
    pub(super) activity_refs: Vec<ActivityRef>,
    pub(super) dropped_claims: Vec<DroppedSource>,
}

pub(super) struct SummaryRow {
    pub(super) id: i64,
    pub(super) user_key: String,
    pub(super) owner_scope: String,
    pub(super) owner_key: String,
    pub(super) scope: String,
    pub(super) scope_key: Option<String>,
    pub(super) summary_text: String,
    pub(super) source_claim_ids_json: String,
    pub(super) source_memory_ids_json: String,
    pub(super) source_activity_refs_json: String,
    pub(super) status: String,
    pub(super) model: Option<String>,
    pub(super) version: i64,
    pub(super) created_at_epoch: i64,
    pub(super) updated_at_epoch: i64,
}

pub(super) struct ClaimCandidate {
    pub(super) id: i64,
    pub(super) claim_type: String,
    pub(super) claim_key: String,
    pub(super) claim_text: String,
    pub(super) owner_scope: String,
    pub(super) owner_key: String,
    pub(super) sensitivity: String,
    pub(super) status: String,
    pub(super) valid_from_epoch: Option<i64>,
    pub(super) valid_to_epoch: Option<i64>,
}
