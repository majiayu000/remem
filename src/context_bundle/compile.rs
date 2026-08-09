//! End-to-end SessionStart compile: request -> plan -> candidates ->
//! bundle (GH-932).
//!
//! This is the first path that reads real data. Everything below the
//! request is deterministic given the database contents: the plan is a
//! pure function of the request, and the executor's selection is a pure
//! function of the plan plus the candidate set.

use anyhow::Result;
use rusqlite::Connection;

use crate::context::{
    load_session_start_candidates_with_limits, ContextLimits, LoadedBundleCandidates,
    SessionStartRelevancePlan,
};
use crate::retrieval::embedding::local_only_embedding_profile_fingerprint;
use crate::retrieval_router::{
    plan_context_bundle_with_limits, plan_session_start_with_limits, RetrievalPlan,
};

use super::domain::{ContextBundle, ContextItem, ContextRequest};
use super::executor::{
    blocked_before_load, execute, execute_with_trace, BudgetEnforcement, ExecutorInputs,
};

pub(crate) struct SessionStartCompile {
    pub bundle: ContextBundle,
    pub relevance_plan: SessionStartRelevancePlan,
}

/// Compile a SessionStart bundle from the database.
///
/// Returns `Err` only when the plan itself cannot be compiled (an invalid
/// request). A canonical load failure is *not* an error return: it
/// produces a `Blocked` bundle so the failure travels with the audit
/// instead of being swallowed or mistaken for an empty project.
pub fn compile_session_start_bundle(
    conn: &Connection,
    request: &ContextRequest,
    cwd: &str,
    current_branch: Option<&str>,
    enrichment_available: bool,
) -> Result<ContextBundle> {
    let limits = ContextLimits::from_env();
    let local_embedding_fingerprint = local_only_embedding_profile_fingerprint();
    let compiled = plan_context_bundle_with_limits(request, &limits, &local_embedding_fingerprint)?;
    Ok(bundle_for_plan(
        conn,
        &compiled,
        &request.project.key,
        cwd,
        current_branch,
        &limits,
        enrichment_available,
    ))
}

#[allow(clippy::too_many_arguments)]
fn bundle_for_plan(
    conn: &Connection,
    compiled: &RetrievalPlan,
    project: &str,
    cwd: &str,
    current_branch: Option<&str>,
    limits: &ContextLimits,
    enrichment_available: bool,
) -> ContextBundle {
    match load_session_start_candidates_with_limits(conn, project, cwd, current_branch, limits) {
        Ok(LoadedBundleCandidates {
            candidates,
            poisoning_drops,
            preselection_drops,
        }) => execute(
            compiled,
            &ExecutorInputs {
                candidates,
                poisoning_drops,
                preselection_drops,
                enrichment_available,
            },
        ),
        Err(error) => blocked_before_load(compiled, &error.to_string()),
    }
}

/// Compile the bundle consumed by the production SessionStart compatibility
/// renderer from the caller's single canonical snapshot.
///
/// Scope, trust, and relevance decisions are final here. Exact item/character
/// budgets are deliberately deferred to the established renderer so adopting
/// the bundle cannot change a byte of host-visible context. The renderer must
/// call [`seal_session_start_bundle`] before the bundle can leave that path.
pub(crate) fn compile_session_start_for_renderer(
    request: &ContextRequest,
    limits: &ContextLimits,
    candidates: Vec<ContextItem>,
    poisoning_drops: Vec<ContextItem>,
    preselection_drops: Vec<super::executor::PreselectionDrop>,
    enrichment_available: bool,
) -> Result<SessionStartCompile> {
    let compiled = plan_session_start_with_limits(request, limits)?;
    let trace = execute_with_trace(
        &compiled,
        &ExecutorInputs {
            candidates,
            poisoning_drops,
            preselection_drops,
            enrichment_available,
        },
        BudgetEnforcement::DeferToRenderer,
    );
    Ok(SessionStartCompile {
        bundle: trace.bundle,
        relevance_plan: trace.relevance_plan,
    })
}

/// Seal a renderer-deferred bundle to the exact identities that survived
/// section and total character budgets.
pub(crate) fn seal_session_start_bundle(
    bundle: &mut ContextBundle,
    selected_keys: &std::collections::HashSet<String>,
    total_truncated_keys: &std::collections::HashSet<String>,
    output_chars: usize,
) {
    for section in [
        &mut bundle.preferences,
        &mut bundle.failure_lessons,
        &mut bundle.current_truth,
        &mut bundle.workstreams,
        &mut bundle.memory_index,
        &mut bundle.recent_sessions,
    ] {
        section.retain(|item| selected_keys.contains(&item.stable_key));
    }
    for entry in &mut bundle.audit.entries {
        if entry.selected && !selected_keys.contains(&entry.stable_key) {
            entry.selected = false;
            entry.reason = if total_truncated_keys.contains(&entry.stable_key) {
                "total_char_limit"
            } else {
                "section_budget"
            }
            .to_string();
        }
    }
    bundle.audit.selected_count = bundle
        .audit
        .entries
        .iter()
        .filter(|entry| entry.selected)
        .count() as u32;
    bundle.audit.dropped_count = bundle.audit.candidates_considered - bundle.audit.selected_count;
    bundle.audit.token_estimate = (output_chars as u32).div_ceil(4);
    if !total_truncated_keys.is_empty() {
        bundle.audit.truncation_reason = Some("total_char_limit".to_string());
    }
}
