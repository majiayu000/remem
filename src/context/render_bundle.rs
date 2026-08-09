//! Context Bundle adapter for the byte-compatible SessionStart renderer.

use std::collections::HashSet;

use anyhow::Result;

use crate::context_bundle::{
    compile_session_start_for_renderer, seal_session_start_bundle, AgentRole, ContextBundle,
    ContextRequest as BundleRequest, ProjectRef, RiskClass, CONTEXT_BUNDLE_SCHEMA_VERSION,
};

use super::format::char_len;
use super::policy::ContextPolicy;
use super::relevance::{memory_stable_key, session_stable_key, SessionStartRelevancePlan};
use super::types::{ContextRequest, LoadedContext};

const CONTEXT_BUNDLE_RENDER_MODE_ENV: &str = "REMEM_CONTEXT_BUNDLE_RENDER_MODE";

pub(super) fn renderer_enabled() -> Result<bool> {
    match std::env::var(CONTEXT_BUNDLE_RENDER_MODE_ENV) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "" | "bundle" => Ok(true),
            "legacy" => Ok(false),
            _ => anyhow::bail!("{CONTEXT_BUNDLE_RENDER_MODE_ENV} must be 'bundle' or 'legacy'"),
        },
        Err(std::env::VarError::NotPresent) => Ok(true),
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("{CONTEXT_BUNDLE_RENDER_MODE_ENV} must be valid Unicode")
        }
    }
}

pub(super) fn compile_for_renderer(
    conn: &rusqlite::Connection,
    loaded: &LoadedContext,
    request: &ContextRequest,
    policy: &ContextPolicy,
    preference_ids: &[i64],
    core_ids: &HashSet<i64>,
) -> Result<(ContextBundle, SessionStartRelevancePlan)> {
    let candidates = super::bundle_candidates::session_start_candidates_from_loaded(
        conn,
        loaded,
        &request.project,
        preference_ids,
        core_ids,
    )?;
    let bundle_request = BundleRequest {
        schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
        task: loaded.relevance_query.clone().unwrap_or_default(),
        project: ProjectRef {
            key: request.project.clone(),
        },
        branch: request.current_branch.clone(),
        worktree: Some(request.cwd.clone()),
        role: AgentRole::Coder,
        as_of_epoch: loaded.render_reference_epoch,
        token_budget: (policy.limits.total_char_limit as u32).div_ceil(4).max(1),
        risk: RiskClass::Low,
        include_superseded: false,
    };
    let compiled =
        compile_session_start_for_renderer(&bundle_request, &policy.limits, candidates, true)?;
    Ok((compiled.bundle, compiled.relevance_plan))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn seal_after_render(
    bundle: &mut ContextBundle,
    preference_ids: &[i64],
    core_ids: &[i64],
    lesson_ids: &[i64],
    index_ids: &[i64],
    session_ids: &[i64],
    workstream_ids: &[i64],
    total_truncated_keys: &HashSet<String>,
    output: &str,
) -> Result<()> {
    let selected_keys = rendered_keys(
        preference_ids,
        core_ids,
        lesson_ids,
        index_ids,
        session_ids,
        workstream_ids,
    );
    let available_keys = item_keys(bundle);
    let missing = selected_keys
        .difference(&available_keys)
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        anyhow::bail!(
            "Context Bundle omitted SessionStart-rendered identities: {}",
            missing.join(", ")
        );
    }
    seal_session_start_bundle(
        bundle,
        &selected_keys,
        total_truncated_keys,
        char_len(output),
    );
    crate::log::debug(
        "context-bundle",
        &format!(
            "sessionstart sealed plan={} selected={} dropped={} mode={:?}",
            bundle.plan_hash,
            bundle.audit.selected_count,
            bundle.audit.dropped_count,
            bundle.degraded_mode
        ),
    );
    Ok(())
}

fn rendered_keys(
    preference_ids: &[i64],
    core_ids: &[i64],
    lesson_ids: &[i64],
    index_ids: &[i64],
    session_ids: &[i64],
    workstream_ids: &[i64],
) -> HashSet<String> {
    preference_ids
        .iter()
        .chain(core_ids)
        .chain(lesson_ids)
        .chain(index_ids)
        .map(|id| memory_stable_key(*id))
        .chain(session_ids.iter().map(|id| session_stable_key(*id)))
        .chain(
            workstream_ids
                .iter()
                .map(|id| super::audit::workstream_stable_key(*id)),
        )
        .collect()
}

fn item_keys(bundle: &ContextBundle) -> HashSet<String> {
    bundle
        .preferences
        .iter()
        .chain(&bundle.failure_lessons)
        .chain(&bundle.current_truth)
        .chain(&bundle.workstreams)
        .chain(&bundle.memory_index)
        .chain(&bundle.recent_sessions)
        .map(|item| item.stable_key.clone())
        .collect()
}
