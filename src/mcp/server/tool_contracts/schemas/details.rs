use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ObservationDetailsOutput {
    details: Vec<DetailOutput>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(untagged)]
enum DetailOutput {
    Memory(MemoryDetailOutput),
    Observation(ObservationDetailOutput),
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MemoryDetailOutput {
    id: i64,
    session_id: Option<String>,
    project: String,
    topic_key: Option<String>,
    title: String,
    text: String,
    memory_type: String,
    files: Option<String>,
    created_at_epoch: i64,
    updated_at_epoch: i64,
    status: String,
    branch: Option<String>,
    scope: String,
    temporal_facts: Option<Vec<MemoryTemporalFactOutput>>,
    topic_trace: Option<Vec<TopicTraceOutput>>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MemoryTemporalFactOutput {
    project: String,
    subject: String,
    predicate: String,
    object: String,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
    learned_at_epoch: i64,
    confidence: f64,
    status: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TopicTraceOutput {
    id: i64,
    topic_key: String,
    title: String,
    summary: String,
    status: String,
    segment_index: i64,
    covered_from_event_id: i64,
    covered_to_event_id: i64,
    evidence_event_ids: Vec<i64>,
    files: Option<Vec<String>>,
    created_at_epoch: i64,
    updated_at_epoch: i64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ObservationDetailOutput {
    id: i64,
    memory_session_id: String,
    #[serde(rename = "type")]
    #[schemars(rename = "type")]
    observation_type: String,
    title: Option<String>,
    subtitle: Option<String>,
    narrative: Option<String>,
    facts: Option<String>,
    concepts: Option<String>,
    files_read: Option<String>,
    files_modified: Option<String>,
    discovery_tokens: Option<i64>,
    created_at: String,
    created_at_epoch: i64,
    project: Option<String>,
    status: String,
    last_accessed_epoch: Option<i64>,
    content_session_id: Option<String>,
    branch: Option<String>,
    commit_sha: Option<String>,
    compressed_sources: Option<Vec<CompressedObservationSourceOutput>>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ObservationOutput {
    id: i64,
    memory_session_id: String,
    #[serde(rename = "type")]
    #[schemars(rename = "type")]
    observation_type: String,
    title: Option<String>,
    subtitle: Option<String>,
    narrative: Option<String>,
    facts: Option<String>,
    concepts: Option<String>,
    files_read: Option<String>,
    files_modified: Option<String>,
    discovery_tokens: Option<i64>,
    created_at: String,
    created_at_epoch: i64,
    project: Option<String>,
    status: String,
    last_accessed_epoch: Option<i64>,
    content_session_id: Option<String>,
    branch: Option<String>,
    commit_sha: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CompressedObservationSourceOutput {
    compressed_observation_id: i64,
    source_observation_id: i64,
    source_hash: String,
    source_created_at_epoch: i64,
    compression_session_id: String,
    created_at_epoch: i64,
}
