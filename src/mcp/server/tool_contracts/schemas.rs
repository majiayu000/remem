#![allow(dead_code)]

mod context_bundle;
mod current_state;
mod details;
mod normalization;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use anyhow::{anyhow, Context};
use rmcp::handler::server::tool::schema_for_output;
use rmcp::model::JsonObject;
use rmcp::schemars::{self, JsonSchema};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use context_bundle::ContextBundleOutput;
use current_state::CurrentStateOutput;
use details::{ObservationDetailsOutput, ObservationOutput};
use normalization::{close_declared_objects, normalize_nullable};

/// Output-only schemas selected by the tool-contract registry.
#[derive(Debug, Clone, Copy)]
pub(super) enum OutputSchema {
    CurrentState,
    Search,
    RecallUserContext,
    ContextBundle,
    Timeline,
    GetObservations,
    LookupCommit,
    CommitsForSession,
    SaveMemory,
    GovernMemory,
    Workstreams,
    UpdateWorkstream,
    SearchRaw,
    ListRawSessions,
}

pub(super) fn build_schema(kind: OutputSchema) -> anyhow::Result<Arc<JsonObject>> {
    let schema = match kind {
        OutputSchema::CurrentState => schema_for_output::<CurrentStateOutput>(),
        OutputSchema::Search => schema_for_output::<SearchOutput>(),
        OutputSchema::RecallUserContext => schema_for_output::<RecallUserContextOutput>(),
        OutputSchema::ContextBundle => schema_for_output::<ContextBundleOutput>(),
        OutputSchema::Timeline => schema_for_output::<TimelineOutput>(),
        OutputSchema::GetObservations => schema_for_output::<ObservationDetailsOutput>(),
        OutputSchema::LookupCommit => schema_for_output::<CommitLookupsOutput>(),
        OutputSchema::CommitsForSession => schema_for_output::<SessionCommitsOutput>(),
        OutputSchema::SaveMemory => schema_for_output::<SaveMemoryOutput>(),
        OutputSchema::GovernMemory => schema_for_output::<GovernMemoryOutput>(),
        OutputSchema::Workstreams => schema_for_output::<WorkstreamsOutput>(),
        OutputSchema::UpdateWorkstream => schema_for_output::<UpdateWorkstreamOutput>(),
        OutputSchema::SearchRaw => schema_for_output::<SearchRawOutput>(),
        OutputSchema::ListRawSessions => schema_for_output::<RawSessionsOutput>(),
    }
    .map_err(|message| anyhow!("failed to build {kind:?} output schema: {message}"))?;

    let schema = normalize_nullable(schema.as_ref().clone())
        .with_context(|| format!("normalize {kind:?} output schema"))?;
    let schema =
        close_declared_objects(schema).with_context(|| format!("close {kind:?} output schema"))?;
    Ok(Arc::new(schema))
}

pub(super) fn validate_output(kind: OutputSchema, value: &Value) -> anyhow::Result<()> {
    match kind {
        OutputSchema::CurrentState => validate::<CurrentStateOutput>(kind, value),
        OutputSchema::Search => validate::<SearchOutput>(kind, value),
        OutputSchema::RecallUserContext => validate::<RecallUserContextOutput>(kind, value),
        OutputSchema::ContextBundle => validate::<ContextBundleOutput>(kind, value),
        OutputSchema::Timeline => validate::<TimelineOutput>(kind, value),
        OutputSchema::GetObservations => validate::<ObservationDetailsOutput>(kind, value),
        OutputSchema::LookupCommit => validate::<CommitLookupsOutput>(kind, value),
        OutputSchema::CommitsForSession => validate::<SessionCommitsOutput>(kind, value),
        OutputSchema::SaveMemory => validate::<SaveMemoryOutput>(kind, value),
        OutputSchema::GovernMemory => validate::<GovernMemoryOutput>(kind, value),
        OutputSchema::Workstreams => validate::<WorkstreamsOutput>(kind, value),
        OutputSchema::UpdateWorkstream => validate::<UpdateWorkstreamOutput>(kind, value),
        OutputSchema::SearchRaw => validate::<SearchRawOutput>(kind, value),
        OutputSchema::ListRawSessions => validate::<RawSessionsOutput>(kind, value),
    }
}

fn validate<T: DeserializeOwned>(kind: OutputSchema, value: &Value) -> anyhow::Result<()> {
    T::deserialize(value)
        .map(drop)
        .with_context(|| format!("validate {kind:?} output DTO"))
}

pub(super) fn deserialize_required_nullable<'de, D, T>(
    deserializer: D,
) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

pub(super) fn required_nullable_string_schema(
    _: &mut schemars::SchemaGenerator,
) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["string", "null"]
    })
}

pub(super) fn required_nullable_number_schema(
    _: &mut schemars::SchemaGenerator,
) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["number", "null"]
    })
}

fn unconstrained_json_value_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    // JSON Schema permits the boolean `true` form for an unconstrained value,
    // but some MCP descriptor consumers accept only object-form schemas.
    // `{}` is semantically equivalent and keeps dynamic extension payloads open.
    schemars::json_schema!({})
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchOutput {
    mode: String,
    results: Vec<SearchResultOutput>,
    next_step: SearchNextStepOutput,
    pagination: SearchPaginationOutput,
    multi_hop: Option<SearchMultiHopOutput>,
    raw_hits: Option<Vec<RawSearchHitOutput>>,
    raw_hits_note: Option<String>,
    raw_hits_error: Option<String>,
    has_more: Option<bool>,
    next_offset: Option<i64>,
    #[schemars(schema_with = "unconstrained_json_value_schema")]
    explain: Option<Value>,
    retrieval_plan: Option<SearchRetrievalPlanOutput>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchRetrievalPlanOutput {
    schema_version: u32,
    policy_version: String,
    plan_hash: String,
    intent: String,
    intent_source: String,
    role: String,
    risk: String,
    reason_codes: Vec<String>,
    applied_effects: Vec<String>,
    filters: SearchRetrievalFiltersOutput,
    rerank_policy: SearchRerankPolicyOutput,
    enabled_channels: Vec<String>,
    disabled_channels: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchRetrievalFiltersOutput {
    project: String,
    branch: Option<String>,
    include_superseded: bool,
    as_of_epoch: i64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchRerankPolicyOutput {
    enabled: bool,
    candidate_pool: u32,
    output_k: u32,
    timeout_fallback: String,
    require_canonical_evidence_top1: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchResultOutput {
    id: i64,
    #[serde(rename = "type")]
    #[schemars(rename = "type")]
    memory_type: String,
    title: String,
    topic_key: Option<String>,
    preview: Option<String>,
    temporal_facts: Option<Vec<String>>,
    source: String,
    source_type: String,
    updated_at: String,
    project: String,
    status: String,
    classification: String,
    classification_reason: String,
    current_context_eligible: bool,
    staleness: Option<MemoryStalenessOutput>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchNextStepOutput {
    tool: String,
    source: String,
    ids: Vec<i64>,
    reason: String,
    include_suppressed: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchPaginationOutput {
    limit: i64,
    offset: i64,
    has_more: bool,
    next_offset: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchMultiHopOutput {
    hops: u8,
    entities_discovered: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawSearchHitOutput {
    id: i64,
    source_type: String,
    session_id: String,
    project: String,
    role: String,
    preview: String,
    source: String,
    branch: Option<String>,
    created_at: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct MemoryStalenessOutput {
    status: String,
    age: String,
    source_anchor: String,
    label: String,
    error: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RecallUserContextOutput {
    query: String,
    project: String,
    task_intent: Option<String>,
    host: Option<String>,
    empty: bool,
    context: String,
    usage_policy: Option<String>,
    included: Vec<RecallIncludedItemOutput>,
    dropped: Vec<RecallDroppedItemOutput>,
    diagnostics: RecallDiagnosticsOutput,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RecallIncludedItemOutput {
    source_type: String,
    source_id: Option<i64>,
    title: Option<String>,
    text: String,
    reason_codes: Vec<String>,
    #[schemars(schema_with = "unconstrained_json_value_schema")]
    source_refs: Option<Value>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RecallDroppedItemOutput {
    source_type: String,
    source_id: Option<i64>,
    label: Option<String>,
    reason_code: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RecallDiagnosticsOutput {
    requested_limit: usize,
    budget_chars: usize,
    used_chars: usize,
    candidate_counts: RecallCandidateCountsOutput,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RecallCandidateCountsOutput {
    summaries: usize,
    claims: usize,
    memories: usize,
    current_state: usize,
    workstreams: usize,
    sessions: usize,
    dropped: usize,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TimelineOutput {
    observations: Vec<ObservationOutput>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CommitLookupsOutput {
    commits: Vec<CommitLookupOutput>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CommitLookupOutput {
    git: GitCommitOutput,
    sessions: Vec<CommitSessionLinkOutput>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SessionCommitsOutput {
    commits: Vec<SessionCommitOutput>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SessionCommitOutput {
    git: GitCommitOutput,
    link: CommitSessionLinkOutput,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GitCommitOutput {
    id: i64,
    project: String,
    repo_path: String,
    sha: String,
    short_sha: String,
    branch: Option<String>,
    message: Option<String>,
    authored_at_epoch: Option<i64>,
    changed_files: Vec<String>,
    created_at_epoch: i64,
    updated_at_epoch: i64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CommitSessionLinkOutput {
    session_id: String,
    memory_session_id: Option<String>,
    source: String,
    linked_at_epoch: i64,
    summary: Option<SessionSummaryTraceOutput>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SessionSummaryTraceOutput {
    request: Option<String>,
    completed: Option<String>,
    decisions: Option<String>,
    learned: Option<String>,
    next_steps: Option<String>,
    preferences: Option<String>,
    created_at_epoch: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SaveMemoryOutput {
    id: i64,
    status: String,
    memory_type: String,
    project: String,
    scope: String,
    topic_key: Option<String>,
    branch: Option<String>,
    operation: String,
    created_at_epoch: i64,
    reference_time_epoch: i64,
    updated_at_epoch: i64,
    upserted: bool,
    local_copy: LocalCopyOutput,
    local_status: String,
    local_path: Option<String>,
    claim_status: String,
    claim_id: Option<i64>,
    claim_error: Option<String>,
    next_step: SaveMemoryNextStepOutput,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LocalCopyOutput {
    status: String,
    path: Option<String>,
    reason: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SaveMemoryNextStepOutput {
    tool: String,
    ids: Vec<i64>,
    source: String,
    reason: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GovernMemoryOutput {
    dry_run: bool,
    action: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    #[schemars(schema_with = "required_nullable_string_schema", required)]
    reason: Option<String>,
    affected: Vec<GovernedMemoryOutput>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GovernedMemoryOutput {
    id: i64,
    title: String,
    previous_status: String,
    new_status: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WorkstreamsOutput {
    workstreams: Vec<WorkstreamOutput>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WorkstreamOutput {
    id: i64,
    project: String,
    title: String,
    description: Option<String>,
    status: WorkstreamStatus,
    progress: Option<String>,
    next_action: Option<String>,
    blockers: Option<String>,
    created_at_epoch: i64,
    updated_at_epoch: i64,
    completed_at_epoch: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(rename_all = "lowercase")]
enum WorkstreamStatus {
    Active,
    Paused,
    Completed,
    Abandoned,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateWorkstreamOutput {
    id: i64,
    updated: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchRawOutput {
    query: String,
    project: Option<String>,
    branch: Option<String>,
    role: Option<String>,
    limit: i64,
    offset: i64,
    since_epoch: Option<i64>,
    until_epoch: Option<i64>,
    source_type: String,
    note: String,
    count: usize,
    has_more: bool,
    next_offset: Option<i64>,
    results: Vec<RawArchiveRowOutput>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawArchiveRowOutput {
    id: i64,
    source_type: String,
    session_id: String,
    project: String,
    role: String,
    content: String,
    source: String,
    branch: Option<String>,
    cwd: Option<String>,
    created_at_epoch: i64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawSessionsOutput {
    since_epoch: Option<i64>,
    until_epoch: Option<i64>,
    project: Option<String>,
    sample: i64,
    count: usize,
    sessions: Vec<RawSessionOutput>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawSessionOutput {
    source_root: String,
    project: String,
    session_id: String,
    first_epoch: i64,
    last_epoch: i64,
    message_count: i64,
    user_message_count: i64,
    assistant_message_count: i64,
    user_message_samples: Vec<String>,
}
