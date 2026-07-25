//! Context Bundle v1 (GH-932): versioned request/plan/bundle/audit contract.
//!
//! remem's callers should face a policy-aware context compiler instead of
//! "several search tools plus one pre-rendered text blob". This module holds
//! the v1 internal Rust contract:
//!
//! - `plan(request)` builds a deterministic [`ContextPlan`] with a stable
//!   plan hash (no timestamps or randomness enter the hash);
//! - `execute(plan, inputs)` selects caller-provided candidates by reusing
//!   the existing SessionStart relevance policy, enforces per-section and
//!   total token budgets, and returns a [`ContextBundle`] whose
//!   [`ContextAudit`] records every considered/selected/dropped candidate
//!   with a machine-readable reason and a degraded mode
//!   (full / canonical_only / blocked).
//!
//! v1 is an experimental internal API. MCP/REST endpoints, DB-backed
//! executor wiring, rerank/graph channels, doctor plan summaries, and
//! benchmark artifact hashes are follow-up work on GH-932.

mod audit;
mod domain;
mod executor;
mod planner;
mod policy;
#[cfg(test)]
mod tests;

pub use domain::{
    AgentRole, AuditEntry, ChannelKind, ContextAudit, ContextBundle, ContextFilters, ContextIntent,
    ContextItem, ContextPlan, ContextRequest, DegradedMode, ItemValidity, PlannedChannel,
    ProjectRef, RiskClass, SectionBudgets, SourceKind, TrustClass, CONTEXT_BUNDLE_SCHEMA_VERSION,
};
pub use executor::{execute, ExecutorInputs};
pub use planner::plan;
pub use policy::CONTEXT_BUNDLE_POLICY_VERSION;
