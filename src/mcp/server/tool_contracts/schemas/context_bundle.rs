use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;

/// Closed output-only mirror of the experimental Context Bundle v1 wire
/// contract. The runtime producer remains `context_bundle::ContextBundle`;
/// deserializing into this mirror makes MCP schema drift fail loudly.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ContextBundleOutput {
    schema_version: u32,
    plan_hash: String,
    degraded_mode: DegradedModeOutput,
    preferences: Vec<ContextItemOutput>,
    failure_lessons: Vec<ContextItemOutput>,
    current_truth: Vec<ContextItemOutput>,
    workstreams: Vec<ContextItemOutput>,
    memory_index: Vec<ContextItemOutput>,
    recent_sessions: Vec<ContextItemOutput>,
    audit: ContextAuditOutput,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ContextItemOutput {
    stable_key: String,
    channel: ChannelKindOutput,
    title: String,
    text: String,
    source_kind: SourceKindOutput,
    canonical_ref: Option<String>,
    projection_ref: Option<String>,
    evidence_refs: Vec<String>,
    validity: ItemValidityOutput,
    trust: TrustClassOutput,
    project: Option<String>,
    branch: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ContextAuditOutput {
    schema_version: u32,
    policy_version: String,
    relevance_policy_version: String,
    plan_hash: String,
    degraded_mode: DegradedModeOutput,
    candidates_considered: u32,
    selected_count: u32,
    dropped_count: u32,
    token_estimate: u32,
    token_budget: u32,
    truncation_reason: Option<String>,
    entries: Vec<AuditEntryOutput>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AuditEntryOutput {
    stable_key: String,
    channel: ChannelKindOutput,
    source_kind: SourceKindOutput,
    validity: ItemValidityOutput,
    selected: bool,
    reason: String,
    relevance_score: Option<f64>,
    token_estimate: u32,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename_all = "snake_case")]
enum DegradedModeOutput {
    Full,
    CanonicalOnly,
    Blocked,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename_all = "snake_case")]
enum ChannelKindOutput {
    Preferences,
    Lessons,
    Core,
    Workstreams,
    MemoryIndex,
    Sessions,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename_all = "snake_case")]
enum SourceKindOutput {
    Canonical,
    Generated,
    GraphDerived,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename_all = "snake_case")]
enum ItemValidityOutput {
    Current,
    Stale,
    Superseded,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename_all = "snake_case")]
enum TrustClassOutput {
    Trusted,
    Standard,
    Quarantined,
}
