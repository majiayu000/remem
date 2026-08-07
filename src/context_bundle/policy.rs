//! Policy version, budget derivation, validation, and machine-readable
//! selection/drop reasons for Context Bundle v1.

use anyhow::{bail, Result};

use crate::context::ContextLimits;

use crate::retrieval_router::{
    RetrievalPlan, RETRIEVAL_PLAN_SCHEMA_VERSION, RETRIEVAL_ROUTER_POLICY_VERSION,
};

use super::domain::{ContextRequest, SectionBudgets, CONTEXT_BUNDLE_SCHEMA_VERSION};

/// Chars-per-token heuristic shared by budgets and item estimates.
const CHARS_PER_TOKEN: u32 = 4;

// Selection reasons.
pub(super) const REASON_SELECTED_CHANNEL: &str = "channel_default_selected";
pub(super) const REASON_SELECTED_RELEVANCE: &str = "relevance_selected";

// Drop reasons (relevance drops reuse the SessionStart reason strings
// `below_sessionstart_relevance_threshold` / `sessionstart_k_limit`).
pub(super) const REASON_QUARANTINED_TRUST: &str = "quarantined_trust";
pub(super) const REASON_PROJECT_SCOPE_MISMATCH: &str = "project_scope_mismatch";
pub(super) const REASON_BRANCH_SCOPE_MISMATCH: &str = "branch_scope_mismatch";
pub(super) const REASON_SUPERSEDED_EXCLUDED: &str = "superseded_excluded";
pub(super) const REASON_CANONICAL_ONLY_DEGRADED: &str = "canonical_only_degraded";
pub(super) const REASON_CHANNEL_ITEM_LIMIT: &str = "channel_item_limit";
pub(super) const REASON_CHANNEL_TOKEN_BUDGET: &str = "channel_token_budget";
pub(super) const REASON_TOTAL_TOKEN_BUDGET: &str = "total_token_budget";
pub(super) const REASON_PLAN_BLOCKED: &str = "plan_blocked";
pub(super) const REASON_CANONICAL_LOAD_FAILED: &str = "canonical_load_failed";

/// Rough token estimate; deterministic and monotonic in text length.
pub(super) fn estimate_tokens(text: &str) -> u32 {
    let chars = text.chars().count() as u32;
    chars.div_ceil(CHARS_PER_TOKEN)
}

/// Planner-side scope validation. Executors re-validate the plan; both
/// layers must agree before any candidate is considered.
pub(crate) fn validate_request(request: &ContextRequest) -> Result<()> {
    if request.schema_version != CONTEXT_BUNDLE_SCHEMA_VERSION {
        bail!(
            "unsupported ContextRequest schema_version {} (expected {})",
            request.schema_version,
            CONTEXT_BUNDLE_SCHEMA_VERSION
        );
    }
    if request.project.key.trim().is_empty() {
        bail!("ContextRequest.project.key must not be empty");
    }
    if request.token_budget == 0 {
        bail!("ContextRequest.token_budget must be greater than zero");
    }
    Ok(())
}

/// Executor-side plan validation. A failure here means canonical scope
/// safety cannot be guaranteed and the bundle must be `Blocked`.
pub(super) fn validate_plan(plan: &RetrievalPlan) -> Result<()> {
    if plan.schema_version != RETRIEVAL_PLAN_SCHEMA_VERSION {
        bail!(
            "unsupported RetrievalPlan schema_version {} (expected {})",
            plan.schema_version,
            RETRIEVAL_PLAN_SCHEMA_VERSION
        );
    }
    if plan.policy_version != RETRIEVAL_ROUTER_POLICY_VERSION {
        bail!(
            "unsupported RetrievalPlan policy_version {:?} (expected {:?})",
            plan.policy_version,
            RETRIEVAL_ROUTER_POLICY_VERSION
        );
    }
    if plan.filters.project.trim().is_empty() {
        bail!("RetrievalPlan.filters.project must not be empty");
    }
    if plan.section_budgets.total_tokens == 0 {
        bail!("RetrievalPlan.section_budgets.total_tokens must be greater than zero");
    }
    if plan.plan_hash.is_empty() {
        bail!("RetrievalPlan.plan_hash must not be empty");
    }
    Ok(())
}

/// Derive per-section token budgets from the compiled SessionStart limits.
///
/// v1 intentionally uses `ContextLimits::default()` rather than the env
/// reader so a plan is a pure function of the request and the compiled
/// policy version; env overrides are follow-up work on GH-932.
pub(crate) fn section_budgets(total_token_budget: u32) -> SectionBudgets {
    let limits = ContextLimits::default();
    let to_tokens = |chars: usize| (chars as u32).div_ceil(CHARS_PER_TOKEN);
    SectionBudgets {
        total_tokens: total_token_budget,
        preferences: to_tokens(limits.preference_char_limit),
        lessons: to_tokens(limits.lesson_char_limit),
        core: to_tokens(limits.core_char_limit),
        workstreams: to_tokens(1_200),
        memory_index: to_tokens(limits.memory_index_char_limit),
        sessions: to_tokens(2_200),
    }
}
