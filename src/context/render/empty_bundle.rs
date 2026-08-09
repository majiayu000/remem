//! Empty-state Context Bundle sealing for SessionStart.

use std::collections::{HashMap, HashSet};

use anyhow::Result;

use super::super::audit::{build_context_audit_items, ContextAuditItem, ContextAuditRenderState};
use super::super::policy::ContextPolicy;
use super::super::types::{ContextRequest, LoadedContext};

pub(super) fn compile_empty_bundle(
    loaded: &LoadedContext,
    request: &ContextRequest,
    policy: &ContextPolicy,
    preference_details: &crate::memory::preference::PreferenceRenderDetails,
    output: &str,
) -> Result<(crate::context_bundle::ContextBundle, Vec<ContextAuditItem>)> {
    let empty_ids = Vec::new();
    let empty_core_ids = HashSet::new();
    let empty_truncated_keys = HashSet::new();
    let empty_ends = HashMap::new();
    let (mut context_bundle, relevance_plan) = super::super::render_bundle::compile_for_renderer(
        loaded,
        request,
        policy,
        preference_details,
        &empty_core_ids,
    )?;
    super::super::render_bundle::seal_after_render(
        &mut context_bundle,
        &empty_ids,
        &empty_ids,
        &empty_ids,
        &empty_ids,
        &empty_ids,
        &empty_ids,
        &empty_truncated_keys,
        output,
    )?;
    let audit_render = ContextAuditRenderState {
        core_selected_ids: &empty_ids,
        core_final_ids: &empty_ids,
        index_final_ids: &empty_ids,
        lesson_final_ids: &empty_ids,
        session_final_ids: &empty_ids,
        workstream_selected_ids: &empty_ids,
        workstream_final_ids: &empty_ids,
        item_end_chars: &empty_ends,
    };
    let audit_items = build_context_audit_items(
        loaded,
        &audit_render,
        &relevance_plan,
        &empty_truncated_keys,
    );
    Ok((context_bundle, audit_items))
}
