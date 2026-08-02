#![allow(dead_code)]

mod current_state;
mod details;
mod normalization;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use anyhow::{anyhow, bail, Context};
use rmcp::handler::server::tool::schema_for_output;
use rmcp::model::JsonObject;
use rmcp::schemars::{self, JsonSchema};
use serde_json::Value;

use current_state::CurrentStateOutput;
use details::{ObservationDetailsOutput, ObservationOutput};
use normalization::normalize_nullable;

/// Output-only schemas selected by the tool-contract registry.
#[derive(Debug, Clone, Copy)]
pub(super) enum OutputSchema {
    CurrentState,
    Search,
    RecallUserContext,
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

    let mut schema = normalize_nullable(schema.as_ref().clone())
        .with_context(|| format!("normalize {kind:?} output schema"))?;
    if matches!(kind, OutputSchema::GovernMemory) {
        require_root_property(&mut schema, "reason")?;
    }
    Ok(Arc::new(schema))
}

fn require_root_property(schema: &mut JsonObject, property: &str) -> anyhow::Result<()> {
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .context("output schema root must contain object properties")?;
    if !properties.contains_key(property) {
        bail!("output schema has no root property {property:?}");
    }

    let required = schema
        .entry("required")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .context("output schema root required must be an array")?;
    if !required
        .iter()
        .any(|value| value.as_str() == Some(property))
    {
        required.push(Value::String(property.to_string()));
    }
    Ok(())
}

#[derive(JsonSchema)]
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
    explain: Option<Value>,
}

#[derive(JsonSchema)]
struct SearchResultOutput {
    id: i64,
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
    staleness: Option<MemoryStalenessOutput>,
}

#[derive(JsonSchema)]
struct SearchNextStepOutput {
    tool: String,
    source: String,
    ids: Vec<i64>,
    reason: String,
    include_suppressed: Option<bool>,
}

#[derive(JsonSchema)]
struct SearchPaginationOutput {
    limit: i64,
    offset: i64,
    has_more: bool,
    next_offset: Option<i64>,
}

#[derive(JsonSchema)]
struct SearchMultiHopOutput {
    hops: u8,
    entities_discovered: Vec<String>,
}

#[derive(JsonSchema)]
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

#[derive(JsonSchema)]
pub(super) struct MemoryStalenessOutput {
    status: String,
    age: String,
    source_anchor: String,
    label: String,
    error: Option<String>,
}

#[derive(JsonSchema)]
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

#[derive(JsonSchema)]
struct RecallIncludedItemOutput {
    source_type: String,
    source_id: Option<i64>,
    title: Option<String>,
    text: String,
    reason_codes: Vec<String>,
    source_refs: Option<Value>,
}

#[derive(JsonSchema)]
struct RecallDroppedItemOutput {
    source_type: String,
    source_id: Option<i64>,
    label: Option<String>,
    reason_code: String,
}

#[derive(JsonSchema)]
struct RecallDiagnosticsOutput {
    requested_limit: usize,
    budget_chars: usize,
    used_chars: usize,
    candidate_counts: RecallCandidateCountsOutput,
}

#[derive(JsonSchema)]
struct RecallCandidateCountsOutput {
    summaries: usize,
    claims: usize,
    memories: usize,
    current_state: usize,
    workstreams: usize,
    sessions: usize,
    dropped: usize,
}

#[derive(JsonSchema)]
struct TimelineOutput {
    observations: Vec<ObservationOutput>,
}

#[derive(JsonSchema)]
struct CommitLookupsOutput {
    commits: Vec<CommitLookupOutput>,
}

#[derive(JsonSchema)]
struct CommitLookupOutput {
    git: GitCommitOutput,
    sessions: Vec<CommitSessionLinkOutput>,
}

#[derive(JsonSchema)]
struct SessionCommitsOutput {
    commits: Vec<SessionCommitOutput>,
}

#[derive(JsonSchema)]
struct SessionCommitOutput {
    git: GitCommitOutput,
    link: CommitSessionLinkOutput,
}

#[derive(JsonSchema)]
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

#[derive(JsonSchema)]
struct CommitSessionLinkOutput {
    session_id: String,
    memory_session_id: Option<String>,
    source: String,
    linked_at_epoch: i64,
    summary: Option<SessionSummaryTraceOutput>,
}

#[derive(JsonSchema)]
struct SessionSummaryTraceOutput {
    request: Option<String>,
    completed: Option<String>,
    decisions: Option<String>,
    learned: Option<String>,
    next_steps: Option<String>,
    preferences: Option<String>,
    created_at_epoch: Option<i64>,
}

#[derive(JsonSchema)]
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

#[derive(JsonSchema)]
struct LocalCopyOutput {
    status: String,
    path: Option<String>,
    reason: Option<String>,
}

#[derive(JsonSchema)]
struct SaveMemoryNextStepOutput {
    tool: String,
    ids: Vec<i64>,
    source: String,
    reason: String,
}

#[derive(JsonSchema)]
struct GovernMemoryOutput {
    dry_run: bool,
    action: String,
    reason: Option<String>,
    affected: Vec<GovernedMemoryOutput>,
}

#[derive(JsonSchema)]
struct GovernedMemoryOutput {
    id: i64,
    title: String,
    previous_status: String,
    new_status: String,
}

#[derive(JsonSchema)]
struct WorkstreamsOutput {
    workstreams: Vec<WorkstreamOutput>,
}

#[derive(JsonSchema)]
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

#[derive(JsonSchema)]
#[schemars(rename_all = "lowercase")]
enum WorkstreamStatus {
    Active,
    Paused,
    Completed,
    Abandoned,
}

#[derive(JsonSchema)]
struct UpdateWorkstreamOutput {
    id: i64,
    updated: bool,
}

#[derive(JsonSchema)]
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

#[derive(JsonSchema)]
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

#[derive(JsonSchema)]
struct RawSessionsOutput {
    since_epoch: Option<i64>,
    until_epoch: Option<i64>,
    project: Option<String>,
    sample: i64,
    count: usize,
    sessions: Vec<RawSessionOutput>,
}

#[derive(JsonSchema)]
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
