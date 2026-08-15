//! Versioned DTOs for the Context Bundle v1 contract (GH-932).
//!
//! Every top-level DTO carries `schema_version` so later revisions can
//! break the shape explicitly. Serialization is serde JSON with snake_case
//! enum values; `tests/schema.rs` pins the exact structure.

use serde::{Deserialize, Serialize};

/// Version of the request/plan/bundle/audit JSON shapes.
pub const CONTEXT_BUNDLE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Coder,
    Reviewer,
    Planner,
    Researcher,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskClass {
    Low,
    Medium,
    High,
}

/// Bounded degradation contract: enrichment/vector/rerank incompatibility
/// may degrade to `CanonicalOnly`; only canonical schema or scope-safety
/// failures produce `Blocked`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradedMode {
    Full,
    CanonicalOnly,
    Blocked,
}

/// Where an item's text came from. Generated or graph-derived projections
/// explain "why it was found" and must never masquerade as canonical
/// memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Canonical,
    Generated,
    GraphDerived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemValidity {
    Current,
    Stale,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustClass {
    Trusted,
    Standard,
    Quarantined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextIntent {
    SessionStart,
    // GH-934 retrieval-router intents. The router compiles a
    // `RetrievalPlan` for these; `SessionStart` stays the bundle-planner
    // intent and is not routable (see `crate::retrieval_router`).
    ResumeWork,
    ExplainDecision,
    DebugFailure,
    ApplyPreference,
    ReviewChange,
    ExploreHistory,
}

/// The retrieval channels the v1 planner knows about; they mirror the
/// SessionStart sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Preferences,
    Lessons,
    Core,
    Workstreams,
    MemoryIndex,
    Sessions,
}

impl ChannelKind {
    /// Deterministic execution/budget order; mirrors the SessionStart
    /// render order (lessons before core before index before sessions).
    pub const ORDERED: [ChannelKind; 6] = [
        ChannelKind::Preferences,
        ChannelKind::Lessons,
        ChannelKind::Core,
        ChannelKind::Workstreams,
        ChannelKind::MemoryIndex,
        ChannelKind::Sessions,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRef {
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextRequest {
    pub schema_version: u32,
    pub task: String,
    pub project: ProjectRef,
    pub branch: Option<String>,
    pub worktree: Option<String>,
    pub role: AgentRole,
    pub as_of_epoch: i64,
    pub token_budget: u32,
    pub risk: RiskClass,
    pub include_superseded: bool,
}

/// One candidate or selected context item. `stable_key` follows the
/// SessionStart identity convention (`memory:<id>`, `session_summary:<id>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextItem {
    pub stable_key: String,
    pub channel: ChannelKind,
    pub title: String,
    pub text: String,
    pub source_kind: SourceKind,
    pub canonical_ref: Option<String>,
    pub projection_ref: Option<String>,
    pub evidence_refs: Vec<String>,
    pub validity: ItemValidity,
    pub trust: TrustClass,
    pub project: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedChannel {
    pub channel: ChannelKind,
    pub item_limit: u32,
    /// Whether the SessionStart relevance selector governs this channel.
    pub relevance_governed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextFilters {
    pub project: String,
    pub branch: Option<String>,
    pub include_superseded: bool,
    pub as_of_epoch: i64,
}

/// Per-section token budgets plus the total request budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionBudgets {
    pub total_tokens: u32,
    pub preferences: u32,
    pub lessons: u32,
    pub core: u32,
    pub workstreams: u32,
    pub memory_index: u32,
    pub sessions: u32,
}

impl SectionBudgets {
    pub fn for_channel(&self, channel: ChannelKind) -> u32 {
        match channel {
            ChannelKind::Preferences => self.preferences,
            ChannelKind::Lessons => self.lessons,
            ChannelKind::Core => self.core,
            ChannelKind::Workstreams => self.workstreams,
            ChannelKind::MemoryIndex => self.memory_index,
            ChannelKind::Sessions => self.sessions,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEntry {
    pub stable_key: String,
    pub channel: ChannelKind,
    pub source_kind: SourceKind,
    pub validity: ItemValidity,
    pub selected: bool,
    /// Machine-readable snake_case reason; see `policy` reason constants
    /// and the SessionStart relevance drop reasons.
    pub reason: String,
    pub relevance_score: Option<f64>,
    pub token_estimate: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextAudit {
    pub schema_version: u32,
    pub policy_version: String,
    pub relevance_policy_version: String,
    pub plan_hash: String,
    pub degraded_mode: DegradedMode,
    pub candidates_considered: u32,
    pub selected_count: u32,
    pub dropped_count: u32,
    pub token_estimate: u32,
    pub token_budget: u32,
    pub truncation_reason: Option<String>,
    pub entries: Vec<AuditEntry>,
    /// G3: Core-channel mapping vs CurrentTruth selected claims, recorded
    /// before activation rewrites the live `current_truth` section.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shadow_comparison: Vec<CurrentTruthShadowDiff>,
}

/// Inclusion/exclusion diff between today's Core channel and CurrentTruth.
/// Not a user-facing section: audit-only, no memory text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentTruthShadowDiff {
    pub stable_key: String,
    /// `core_only`, `projection_only`, or `abstained`.
    pub verdict: String,
    pub projection_ref: Option<String>,
    pub claim_refs: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextBundle {
    pub schema_version: u32,
    pub plan_hash: String,
    pub degraded_mode: DegradedMode,
    pub preferences: Vec<ContextItem>,
    pub failure_lessons: Vec<ContextItem>,
    pub current_truth: Vec<ContextItem>,
    pub workstreams: Vec<ContextItem>,
    pub memory_index: Vec<ContextItem>,
    pub recent_sessions: Vec<ContextItem>,
    pub audit: ContextAudit,
}

impl ContextBundle {
    pub fn section_mut(&mut self, channel: ChannelKind) -> &mut Vec<ContextItem> {
        match channel {
            ChannelKind::Preferences => &mut self.preferences,
            ChannelKind::Lessons => &mut self.failure_lessons,
            ChannelKind::Core => &mut self.current_truth,
            ChannelKind::Workstreams => &mut self.workstreams,
            ChannelKind::MemoryIndex => &mut self.memory_index,
            ChannelKind::Sessions => &mut self.recent_sessions,
        }
    }
}
