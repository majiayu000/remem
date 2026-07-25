//! Deterministic v1 planner: wraps the SessionStart selection policy into
//! a versioned [`ContextPlan`] with a stable content hash.

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::context::{ContextLimits, SESSIONSTART_RELEVANCE_POLICY_VERSION};

use super::domain::{
    ChannelKind, ContextFilters, ContextIntent, ContextPlan, ContextRequest, PlannedChannel,
    CONTEXT_BUNDLE_SCHEMA_VERSION,
};
use super::policy::{section_budgets, validate_request, CONTEXT_BUNDLE_POLICY_VERSION};

/// Build a deterministic plan for the request.
///
/// The plan is a pure function of the request plus the compiled policy:
/// no clock reads, no randomness, no env access. Identical requests
/// therefore always produce identical plan JSON and `plan_hash`.
pub fn plan(request: &ContextRequest) -> Result<ContextPlan> {
    validate_request(request)?;
    let limits = ContextLimits::default();
    let relevance_query = normalized_relevance_query(&request.task);
    let mut plan = ContextPlan {
        schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
        policy_version: CONTEXT_BUNDLE_POLICY_VERSION.to_string(),
        relevance_policy_version: SESSIONSTART_RELEVANCE_POLICY_VERSION.to_string(),
        intent: ContextIntent::SessionStart,
        relevance_query,
        relevance_k: limits.sessionstart_relevance_k as u32,
        channels: planned_channels(&limits),
        filters: ContextFilters {
            project: request.project.key.clone(),
            branch: request.branch.clone(),
            include_superseded: request.include_superseded,
            as_of_epoch: request.as_of_epoch,
        },
        section_budgets: section_budgets(request.token_budget),
        plan_hash: String::new(),
    };
    plan.plan_hash = plan_content_hash(&plan)?;
    Ok(plan)
}

fn normalized_relevance_query(task: &str) -> Option<String> {
    let trimmed = task.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn planned_channels(limits: &ContextLimits) -> Vec<PlannedChannel> {
    // Mirrors the SessionStart sections: preferences/core/workstreams are
    // not relevance-governed; lessons/memory_index/sessions are.
    vec![
        PlannedChannel {
            channel: ChannelKind::Preferences,
            item_limit: (limits.preference_project_limit + limits.preference_global_limit) as u32,
            relevance_governed: false,
        },
        PlannedChannel {
            channel: ChannelKind::Lessons,
            item_limit: limits.lesson_limit as u32,
            relevance_governed: true,
        },
        PlannedChannel {
            channel: ChannelKind::Core,
            item_limit: limits.core_item_limit as u32,
            relevance_governed: false,
        },
        PlannedChannel {
            channel: ChannelKind::Workstreams,
            item_limit: 5,
            relevance_governed: false,
        },
        PlannedChannel {
            channel: ChannelKind::MemoryIndex,
            item_limit: limits.memory_index_limit as u32,
            relevance_governed: true,
        },
        PlannedChannel {
            channel: ChannelKind::Sessions,
            item_limit: limits.session_limit as u32,
            relevance_governed: true,
        },
    ]
}

/// SHA-256 hex over the canonical serde JSON of the plan with an empty
/// `plan_hash` field. serde JSON struct field order is declaration order,
/// so the byte stream is stable for a fixed schema version.
pub(super) fn plan_content_hash(plan: &ContextPlan) -> Result<String> {
    let mut hashable = plan.clone();
    hashable.plan_hash = String::new();
    let canonical = serde_json::to_string(&hashable)?;
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    Ok(format!("{:x}", hasher.finalize()))
}
