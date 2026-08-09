//! Context Bundle v1 (GH-932): versioned request/bundle/audit contract.
//!
//! remem's callers should face a policy-aware context compiler instead of
//! "several search tools plus one pre-rendered text blob". This module holds
//! the v1 internal Rust contract:
//!
//! - the plan is compiled by [`crate::retrieval_router::plan`], which owns
//!   the single [`crate::retrieval_router::RetrievalPlan`] type covering
//!   both the retrieval-source side (channels, weights, trust floors) and
//!   the output-section side (sections, item limits, budgets), under one
//!   stable plan hash;
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
mod compile;
mod domain;
mod executor;
mod policy;
#[cfg(test)]
mod tests;

pub use compile::compile_session_start_bundle;
pub(crate) use compile::{compile_session_start_for_renderer, seal_session_start_bundle};
pub use domain::{
    AgentRole, AuditEntry, ChannelKind, ContextAudit, ContextBundle, ContextFilters, ContextIntent,
    ContextItem, ContextRequest, DegradedMode, ItemValidity, PlannedChannel, ProjectRef, RiskClass,
    SectionBudgets, SourceKind, TrustClass, CONTEXT_BUNDLE_SCHEMA_VERSION,
};
pub use executor::{blocked_before_load, execute, ExecutorInputs};
pub(crate) use policy::{section_budgets, section_budgets_from_limits, validate_request};
