use serde::Serialize;

use crate::memory::{Memory, MemoryStalenessLabel};

#[derive(Debug, Clone, Default)]
pub struct CurrentStateRequest {
    pub state_key: String,
    pub project: Option<String>,
    pub owner_scope: Option<String>,
    pub owner_key: Option<String>,
    pub memory_type: Option<String>,
    pub as_of_epoch: Option<i64>,
    pub include_history: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrentStateResult {
    pub status: String,
    pub state_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of_epoch: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<CurrentStateKeySummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub matches: Vec<CurrentStateKeySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<CurrentStateAnswer>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<CurrentStateMemoryRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<CurrentStateMemoryRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<CurrentStateFact>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub why: Vec<CurrentStateWhy>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrentStateKeySummary {
    pub id: i64,
    pub owner_scope: String,
    pub owner_key: String,
    pub memory_type: String,
    pub state_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_label: Option<String>,
    pub state_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_memory_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrentStateAnswer {
    pub id: i64,
    pub title: String,
    pub text: String,
    pub memory_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    pub project: String,
    pub scope: String,
    pub status: String,
    pub updated_at_epoch: i64,
    pub staleness: MemoryStalenessLabel,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrentStateMemoryRef {
    pub id: i64,
    pub title: String,
    pub memory_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    pub project: String,
    pub status: String,
    pub updated_at_epoch: i64,
    pub staleness: MemoryStalenessLabel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub evidence_event_ids: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_candidate_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_operation_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrentStateWhy {
    pub edge_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_memory_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_memory_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub evidence_event_ids: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_candidate_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_operation_id: Option<i64>,
    pub created_at_epoch: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrentStateFact {
    pub id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from_epoch: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to_epoch: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_memory_id: Option<i64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub source_event_ids: Vec<i64>,
    pub status: String,
}

#[derive(Debug, Clone)]
pub(super) struct CurrentStateMemoryRefParts {
    pub(super) memory: Memory,
    pub(super) relation: Option<String>,
    pub(super) reason: Option<String>,
    pub(super) evidence_event_ids: Vec<i64>,
    pub(super) source_candidate_id: Option<i64>,
    pub(super) source_operation_id: Option<i64>,
}

impl CurrentStateAnswer {
    pub(super) fn from_memory(memory: Memory, staleness: MemoryStalenessLabel) -> Self {
        Self {
            id: memory.id,
            title: memory.title,
            text: memory.text,
            memory_type: memory.memory_type,
            topic_key: memory.topic_key,
            project: memory.project,
            scope: memory.scope,
            status: memory.status,
            updated_at_epoch: memory.updated_at_epoch,
            staleness,
        }
    }
}

impl CurrentStateMemoryRef {
    pub(super) fn from_memory(memory: Memory, staleness: MemoryStalenessLabel) -> Self {
        Self {
            id: memory.id,
            title: memory.title,
            memory_type: memory.memory_type,
            topic_key: memory.topic_key,
            project: memory.project,
            status: memory.status,
            updated_at_epoch: memory.updated_at_epoch,
            staleness,
            relation: None,
            reason: None,
            evidence_event_ids: Vec::new(),
            source_candidate_id: None,
            source_operation_id: None,
        }
    }

    pub(super) fn from_parts(
        parts: CurrentStateMemoryRefParts,
        staleness: MemoryStalenessLabel,
    ) -> Self {
        let memory = parts.memory;
        Self {
            id: memory.id,
            title: memory.title,
            memory_type: memory.memory_type,
            topic_key: memory.topic_key,
            project: memory.project,
            status: memory.status,
            updated_at_epoch: memory.updated_at_epoch,
            staleness,
            relation: parts.relation,
            reason: parts.reason,
            evidence_event_ids: parts.evidence_event_ids,
            source_candidate_id: parts.source_candidate_id,
            source_operation_id: parts.source_operation_id,
        }
    }
}
