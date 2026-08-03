use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;

use super::MemoryStalenessOutput;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CurrentStateOutput {
    status: CurrentStateStatus,
    state_key: String,
    as_of_epoch: Option<i64>,
    state: Option<CurrentStateKeySummaryOutput>,
    matches: Option<Vec<CurrentStateKeySummaryOutput>>,
    current: Option<CurrentStateAnswerOutput>,
    conflicts: Option<Vec<CurrentStateMemoryRefOutput>>,
    history: Option<Vec<CurrentStateMemoryRefOutput>>,
    facts: Option<Vec<CurrentStateFactOutput>>,
    why: Option<Vec<CurrentStateWhyOutput>>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename_all = "snake_case")]
enum CurrentStateStatus {
    Current,
    NotFound,
    Ambiguous,
    UnresolvedConflict,
    NoCurrent,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CurrentStateKeySummaryOutput {
    id: i64,
    owner_scope: String,
    owner_key: String,
    memory_type: String,
    state_key: String,
    state_label: Option<String>,
    state_status: String,
    current_memory_id: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CurrentStateAnswerOutput {
    id: i64,
    title: String,
    text: String,
    memory_type: String,
    topic_key: Option<String>,
    project: String,
    scope: String,
    status: String,
    updated_at_epoch: i64,
    staleness: MemoryStalenessOutput,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CurrentStateMemoryRefOutput {
    id: i64,
    title: String,
    memory_type: String,
    topic_key: Option<String>,
    project: String,
    status: String,
    updated_at_epoch: i64,
    staleness: MemoryStalenessOutput,
    relation: Option<String>,
    reason: Option<String>,
    evidence_event_ids: Option<Vec<i64>>,
    source_candidate_id: Option<i64>,
    source_operation_id: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CurrentStateFactOutput {
    id: i64,
    subject: String,
    predicate: String,
    object: String,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
    source_memory_id: Option<i64>,
    source_event_ids: Option<Vec<i64>>,
    status: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CurrentStateWhyOutput {
    edge_type: String,
    from_memory_id: Option<i64>,
    to_memory_id: Option<i64>,
    reason: Option<String>,
    evidence_event_ids: Option<Vec<i64>>,
    source_candidate_id: Option<i64>,
    source_operation_id: Option<i64>,
    created_at_epoch: i64,
}
