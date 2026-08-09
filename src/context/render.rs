use std::collections::HashSet;
use std::time::Instant;

use anyhow::Result;

use crate::db;

use super::audit::{
    build_context_audit_items, record_context_injection_items, ContextAuditItem,
    ContextAuditRenderState,
};
use super::format::char_len;
use super::hook_warning::{append_hook_integrity_warning, claude_hook_integrity_warning};
use super::host::resolve_profile;
use super::injection_gate::{
    apply_context_gate_with_data_version, compute_data_version_hint, pre_render_context_gate,
    ContextGateAction, ContextGateDecision, ContextGatePrecheck,
};
use super::invocation::{
    direct_context_invocation, resolve_context_invocation, resolve_cursor_context_invocation,
    ContextCliOptions, ContextInvocation,
};
use super::policy::{ContextPolicy, SectionKind};
use super::relevance::{build_sessionstart_relevance_plan, candidates_for_loaded, selected_inputs};
use super::render_inputs::{load_context_render_inputs, ContextRenderInputs};
use super::sections::{
    empty_state_output, render_core_memory_with_limits_and_staleness,
    render_lessons_with_summary_and_staleness, render_memory_index_with_summary_and_staleness,
    render_ranked_memory_index_with_summary_and_staleness, render_recent_sessions_with_summary,
    render_workstreams_with_summary,
};
use super::types::{ContextLoadError, ContextRequest};
mod eval;
mod finalize;
pub(in crate::context) mod helpers;
mod stats;
mod timer;
mod truncation;
pub(in crate::context) use super::render_error::render_context_load_errors;
pub(crate) use eval::{governance_eval_snapshot, session_start_eval_snapshot};
use finalize::{finalize_context_output, RenderedIdentityBounds};
use helpers::{
    build_context_header_with_style, context_debug_enabled, context_source_note,
    section_render_limits,
};
#[cfg(test)]
pub(in crate::context) use helpers::{build_context_stats_footer, enforce_total_char_limit};
pub(in crate::context) use helpers::{
    context_stdout_for_invocation, enforce_total_char_limit_preserving_footer,
};
pub(in crate::context) use stats::{ContextRenderStats, SectionRenderStats};
use timer::log_context_timer;

pub(in crate::context) use super::debug::build_context_debug_trace;

pub(crate) const RENDER_CONTRACT_VERSION: u32 = 3;

pub(in crate::context) struct RenderedContext {
    pub(in crate::context) output: String,
    pub(in crate::context) stats: ContextRenderStats,
    pub(in crate::context) audit_items: Vec<ContextAuditItem>,
    pub(in crate::context) data_version: Option<String>,
    pub(in crate::context) has_load_errors: bool,
    #[cfg(test)]
    pub(in crate::context) context_bundle: Option<crate::context_bundle::ContextBundle>,
}

pub fn generate_context(
    cwd: &str,
    session_id: Option<&str>,
    use_colors: bool,
    host_arg: Option<&str>,
    debug: bool,
) -> Result<()> {
    let invocation = direct_context_invocation(cwd, session_id, use_colors, host_arg, debug);
    generate_context_for_invocation(invocation, false)
}

pub fn generate_context_from_cli(
    cwd: Option<String>,
    session_id: Option<String>,
    use_colors: bool,
    host: Option<String>,
    debug: bool,
    force: bool,
    gate_mode: Option<String>,
) -> Result<()> {
    let invocation = resolve_context_invocation(ContextCliOptions {
        cwd,
        session_id,
        host,
        use_colors,
        debug,
        force,
        gate_mode,
    })?;
    generate_context_for_invocation(invocation, true)
}

/// Cursor `remem context` entrypoint (GH-823): bounded stdin read, strict
/// exact `sessionStart` validation, then the shared render pipeline. Any
/// parse/limit failure returns before context generation with empty stdout
/// and no side effects (B-009); no CLI/current-cwd fallback exists.
pub fn generate_cursor_context_from_stdin() -> Result<()> {
    let bytes = crate::cursor_hook::input::read_bounded_hook_stdin(&mut std::io::stdin().lock())?;
    generate_cursor_context_from_bytes(&bytes)
}

pub fn generate_cursor_context_from_bytes(bytes: &[u8]) -> Result<()> {
    let event = crate::cursor_hook::input::parse_session_start(bytes)?;
    let invocation = resolve_cursor_context_invocation(&event);
    generate_context_for_invocation(invocation, true)
}

fn generate_context_for_invocation(invocation: ContextInvocation, use_gate: bool) -> Result<()> {
    let stdout = generate_context_output_for_invocation(invocation, use_gate)?;
    print!("{stdout}");
    Ok(())
}

fn generate_context_output_for_invocation(
    invocation: ContextInvocation,
    use_gate: bool,
) -> Result<String> {
    let timer = crate::log::Timer::start("context", &format!("cwd={}", invocation.cwd));
    let debug_enabled = invocation.debug || context_debug_enabled();
    let request = ContextRequest {
        cwd: invocation.cwd.clone(),
        project: invocation.project.clone(),
        session_id: invocation.session_id.clone(),
        hook_source: invocation.source.clone(),
        current_branch: db::detect_git_branch(&invocation.cwd),
        host: invocation.host,
        use_colors: invocation.use_colors,
    };
    let policy = resolve_profile(request.host).default_policy();
    let hook_integrity_warning = claude_hook_integrity_warning(&invocation);
    let db_open_start = Instant::now();
    let conn = match open_context_connection_or_error(&request, &policy) {
        Ok(conn) => conn,
        Err(rendered) => {
            // GH-823 B-005: Cursor never receives a fallback/half-rendered
            // context body; failure means empty stdout plus an error log.
            if invocation.host == super::host::HostKind::Cursor {
                crate::log::error(
                    "context",
                    "cursor context generation failed (database open); emitting no stdout",
                );
                anyhow::bail!("cursor context generation failed (database open)");
            }
            let rendered = *rendered;
            let mut decision = if use_gate {
                ContextGateDecision {
                    output: rendered.output,
                    action: ContextGateAction::FailOpen,
                    reason: "db_open",
                    key: None,
                    context_hash: None,
                    output_mode: None,
                    retained_context_chars: None,
                }
            } else {
                ContextGateDecision {
                    output: rendered.output,
                    action: ContextGateAction::Bypassed,
                    reason: "legacy_direct",
                    key: None,
                    context_hash: None,
                    output_mode: None,
                    retained_context_chars: None,
                }
            };
            if debug_enabled {
                let decision_for_debug = decision.clone();
                append_context_gate_debug_trace(
                    &mut decision.output,
                    &request,
                    &decision_for_debug,
                );
            }
            append_hook_integrity_warning(&mut decision.output, hook_integrity_warning.as_deref());
            let stdout = context_stdout_for_invocation(&decision.output, &invocation)?;
            log_context_timer(
                timer,
                &request,
                &decision,
                &rendered.stats,
                ContextGatePrecheck::Off,
            );
            return Ok(stdout);
        }
    };
    let db_open_timing = crate::perf::PhaseTiming::elapsed("db_open", db_open_start);
    let (mut decision, mut stats, precheck, audit_items) = if use_gate {
        let precheck_start = Instant::now();
        let precheck =
            pre_render_context_gate(&conn, &invocation, &request, &policy, debug_enabled);
        let precheck_timing = crate::perf::PhaseTiming::elapsed("pre_render_gate", precheck_start);
        if let Some(decision) = precheck.decision {
            let mut stats = ContextRenderStats::default();
            stats.timings.push(db_open_timing.clone());
            stats.timings.push(precheck_timing);
            (decision, stats, precheck.precheck, Vec::new())
        } else {
            let prechecked_data_version = precheck.data_version.clone();
            let rendered = render_context_output_with_policy(
                &conn,
                &request,
                Some(&invocation),
                debug_enabled,
                policy,
                prechecked_data_version,
            )?;
            let mut stats = rendered.stats;
            stats.timings.insert(0, precheck_timing);
            stats.timings.insert(0, db_open_timing.clone());
            let output = rendered.output;
            let data_version = rendered.data_version;
            let has_load_errors = rendered.has_load_errors;
            let audit_items = rendered.audit_items;
            if has_load_errors && invocation.host == super::host::HostKind::Cursor {
                crate::log::error(
                    "context",
                    "cursor context load errors; emitting no stdout (B-005)",
                );
                anyhow::bail!("cursor context generation failed (context load errors)");
            }
            let decision = if has_load_errors {
                ContextGateDecision {
                    output,
                    action: ContextGateAction::FailOpen,
                    reason: "context_load_errors",
                    key: None,
                    context_hash: None,
                    output_mode: Some("fail_open"),
                    retained_context_chars: None,
                }
            } else {
                let gate_start = Instant::now();
                let decision = apply_context_gate_with_data_version(
                    &conn,
                    &invocation,
                    output,
                    data_version.as_deref(),
                );
                stats
                    .timings
                    .push(crate::perf::PhaseTiming::elapsed("gate_apply", gate_start));
                decision
            };
            (decision, stats, precheck.precheck, audit_items)
        }
    } else {
        let rendered =
            render_context_output_with_policy(&conn, &request, None, debug_enabled, policy, None)?;
        let mut stats = rendered.stats;
        stats.timings.insert(0, db_open_timing.clone());
        (
            ContextGateDecision {
                output: rendered.output,
                action: ContextGateAction::Bypassed,
                reason: "legacy_direct",
                key: None,
                context_hash: None,
                output_mode: None,
                retained_context_chars: None,
            },
            stats,
            ContextGatePrecheck::Off,
            rendered.audit_items,
        )
    };
    if debug_enabled {
        let decision_for_debug = decision.clone();
        append_context_gate_debug_trace(&mut decision.output, &request, &decision_for_debug);
    }
    append_hook_integrity_warning(&mut decision.output, hook_integrity_warning.as_deref());
    if !audit_items.is_empty() {
        let audit_write_start = Instant::now();
        if let Err(error) =
            record_context_injection_items(&conn, &invocation, &decision, &audit_items)
        {
            crate::log::warn(
                "context-audit",
                &format!("failed to write audit rows: {error}"),
            );
        }
        stats.timings.push(crate::perf::PhaseTiming::elapsed(
            "audit_write",
            audit_write_start,
        ));
    }
    let stdout = context_stdout_for_invocation(&decision.output, &invocation)?;
    log_context_timer(timer, &request, &decision, &stats, precheck);
    Ok(stdout)
}

#[cfg(test)]
pub(in crate::context) fn generate_context_for_test(
    invocation: ContextInvocation,
    use_gate: bool,
) -> Result<()> {
    generate_context_for_invocation(invocation, use_gate)
}

#[cfg(test)]
pub(in crate::context) fn generate_context_output_for_test(
    invocation: ContextInvocation,
    use_gate: bool,
) -> Result<String> {
    generate_context_output_for_invocation(invocation, use_gate)
}

pub(in crate::context) fn append_context_gate_debug_trace(
    output: &mut String,
    request: &ContextRequest,
    decision: &ContextGateDecision,
) {
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str("## Debug Trace\n");
    output.push_str(&format!(
        "- request host={} project={} cwd={} branch={} session={} source={}\n",
        request.host.as_env_value(),
        request.project,
        request.cwd,
        request.current_branch.as_deref().unwrap_or("-"),
        request.session_id.as_deref().unwrap_or("-"),
        request.hook_source.as_deref().unwrap_or("-")
    ));
    output.push_str(&format!(
        "- gate action={:?} reason={} output_mode={} key={} hash={}\n",
        decision.action,
        decision.reason,
        decision.output_mode.unwrap_or("-"),
        decision.key.as_deref().unwrap_or("-"),
        decision.context_hash.as_deref().unwrap_or("-")
    ));
}

#[cfg(test)]
pub(in crate::context) fn render_context_output(
    request: &ContextRequest,
    debug: bool,
) -> Result<RenderedContext> {
    let profile = resolve_profile(request.host);
    let policy = profile.default_policy();
    let conn = match open_context_connection_or_error(request, &policy) {
        Ok(conn) => conn,
        Err(rendered) => return Ok(*rendered),
    };
    render_context_output_with_policy(&conn, request, None, debug, policy, None)
}

fn render_context_output_with_policy(
    conn: &rusqlite::Connection,
    request: &ContextRequest,
    invocation: Option<&ContextInvocation>,
    debug: bool,
    policy: ContextPolicy,
    prechecked_data_version: Option<String>,
) -> Result<RenderedContext> {
    let inputs = load_context_render_inputs(conn, request, debug, &policy);
    render_context_output_from_inputs(
        conn,
        request,
        invocation,
        debug,
        policy,
        inputs,
        prechecked_data_version,
        super::render_bundle::renderer_enabled()?,
    )
}

pub(in crate::context) fn render_context_output_from_inputs(
    conn: &rusqlite::Connection,
    request: &ContextRequest,
    invocation: Option<&ContextInvocation>,
    debug: bool,
    policy: ContextPolicy,
    inputs: ContextRenderInputs,
    prechecked_data_version: Option<String>,
    use_context_bundle: bool,
) -> Result<RenderedContext> {
    let render_total_start = Instant::now();
    let profile = resolve_profile(request.host);
    let mut loaded = inputs.loaded;
    let has_load_errors = !loaded.errors.is_empty();
    let data_version = if has_load_errors {
        None
    } else {
        prechecked_data_version.or_else(|| {
            invocation.and_then(|invocation| {
                compute_data_version_hint(conn, request, invocation, &policy)
                    .map_err(|error| {
                        crate::log::warn(
                            "context-gate",
                            &format!("render_data_version_skip error={error}"),
                        );
                        error
                    })
                    .ok()
            })
        })
    };
    let preference_output = inputs.preference_output;
    let preference_details = inputs.preference_details;
    let load_timing = inputs.load_timing;
    let preference_timing = inputs.preference_timing;
    let preference_summary = preference_details.summary;
    super::poisoning::drop_unacknowledged_poisoned_context(conn, &mut loaded);

    if preference_summary.rendered == 0
        && loaded.memories.is_empty()
        && loaded.lessons.is_empty()
        && loaded.summaries.is_empty()
        && loaded.workstreams.is_empty()
        && loaded.errors.is_empty()
    {
        return Ok(RenderedContext {
            output: empty_context_output(request),
            stats: stats::empty_stats_with_load(request, &loaded, load_timing, preference_timing),
            audit_items: Vec::new(),
            data_version,
            has_load_errors,
            #[cfg(test)]
            context_bundle: None,
        });
    }

    let mut output = String::new();
    let (mut lesson_ids, mut index_ids, mut session_ids, mut workstream_ids) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut lesson_item_ends, mut index_item_ends, mut session_item_ends) =
        (Vec::new(), Vec::new(), Vec::new());
    let mut workstream_item_ends = Vec::new();
    output.push_str(&build_context_header_with_style(
        &request.project,
        request.current_branch.as_deref(),
        request.hook_source.as_deref(),
        request.host,
        request.use_colors,
    ));
    output.push_str(profile.retrieval_hints().line);
    output.push('\n');
    output.push_str(crate::memory::usage::citation_contract_line());
    output.push('\n');
    output.push_str(crate::user_context::usage_policy::USER_CONTEXT_USAGE_POLICY);
    output.push('\n');
    if let Some(note) = context_source_note(request.hook_source.as_deref()) {
        output.push('\n');
        output.push_str(note);
        output.push('\n');
    }
    output.push('\n');

    render_context_load_errors(&mut output, &loaded.errors);
    let mut stats = ContextRenderStats {
        host: request.host.as_env_value().to_string(),
        branch: request.current_branch.clone(),
        hook_source: request.hook_source.clone(),
        total_char_limit: policy.limits.total_char_limit,
        memories_loaded: loaded.memories.len() + loaded.lessons.len(),
        project_preferences: preference_summary.project_rendered,
        global_preferences: preference_summary.global_rendered,
        owner_counts: {
            let mut counts = loaded.owner_counts.clone();
            counts.add_repo(preference_summary.project_rendered);
            counts.add_user(preference_summary.global_rendered);
            counts
        },
        ..ContextRenderStats::default()
    };
    stats::absorb_evidence(
        &mut stats,
        load_timing,
        &loaded.load_phase_timings,
        preference_timing,
        loaded.rerank.as_ref(),
    );

    let section_start = Instant::now();
    let before = char_len(&output);
    let preference_output_start = before;
    output.push_str(&preference_output);
    let preference_item_ends = preference_details.absolute_item_end_chars(preference_output_start);
    stats.preferences = SectionRenderStats {
        count: preference_summary.rendered,
        chars: char_len(&output).saturating_sub(before),
    };
    stats.timings.push(crate::perf::PhaseTiming::elapsed(
        "section_preferences",
        section_start,
    ));

    let render_limits = section_render_limits(&policy);
    let section_start = Instant::now();
    let mut core_output = String::new();
    let core_summary = render_core_memory_with_limits_and_staleness(
        &mut core_output,
        &loaded.memories,
        &render_limits,
        loaded.render_reference_epoch,
        &loaded.staleness_labels,
    );
    stats.core_ids = core_summary.ids.clone();
    let core_item_ends = core_summary.item_end_chars.clone();
    stats.core = SectionRenderStats {
        count: core_summary.count,
        chars: char_len(&core_output),
    };
    stats.timings.push(crate::perf::PhaseTiming::elapsed(
        "section_core",
        section_start,
    ));
    let core_ids = core_summary.ids.into_iter().collect::<HashSet<_>>();
    let mut context_bundle = None;
    let relevance_plan = if use_context_bundle && !has_load_errors {
        let (bundle, relevance_plan) = super::render_bundle::compile_for_renderer(
            &loaded,
            request,
            &policy,
            &preference_details.rendered_memories,
            &core_ids,
        )?;
        context_bundle = Some(bundle);
        relevance_plan
    } else {
        let relevance_candidates = candidates_for_loaded(&loaded, &core_ids);
        build_sessionstart_relevance_plan(
            loaded.relevance_query.as_deref(),
            policy.limits.sessionstart_relevance_k,
            &relevance_candidates,
        )
    };
    let governed = selected_inputs(&loaded, &relevance_plan, &core_ids)?;
    stats.relevance.state = relevance_plan.state;
    stats.relevance.k = relevance_plan.k;
    stats.relevance.threshold = relevance_plan.threshold;
    stats.relevance.candidates = relevance_plan.candidate_count;
    stats.relevance.eligible = relevance_plan.eligible_count;
    stats.relevance.below_threshold = relevance_plan.below_threshold_count;
    stats.relevance.k_limited = relevance_plan.k_limited_count;

    if !governed.lessons.is_empty() {
        let section_start = Instant::now();
        let before = char_len(&output);
        let lesson_summary = render_lessons_with_summary_and_staleness(
            &mut output,
            &governed.lessons,
            policy.section_item_limit(SectionKind::Lessons, policy.limits.lesson_limit),
            policy.section_char_limit(SectionKind::Lessons, policy.limits.lesson_char_limit),
            loaded.render_reference_epoch,
            &loaded.staleness_labels,
        );
        lesson_ids = lesson_summary.ids;
        lesson_item_ends = lesson_summary.item_end_chars;
        stats.lessons = SectionRenderStats {
            count: lesson_summary.count,
            chars: char_len(&output).saturating_sub(before),
        };
        stats.timings.push(crate::perf::PhaseTiming::elapsed(
            "section_lessons",
            section_start,
        ));
    }
    let core_output_start = char_len(&output);
    output.push_str(&core_output);
    if !governed.memories.is_empty() {
        let section_start = Instant::now();
        let before = char_len(&output);
        let index_summary = if relevance_plan.state == "disabled" {
            render_memory_index_with_summary_and_staleness(
                &mut output,
                &governed.memories,
                &render_limits,
                &core_ids,
                loaded.render_reference_epoch,
                &loaded.staleness_labels,
            )
        } else {
            render_ranked_memory_index_with_summary_and_staleness(
                &mut output,
                &governed.memories,
                &render_limits,
                &core_ids,
                loaded.render_reference_epoch,
                &loaded.staleness_labels,
            )
        };
        index_ids = index_summary.ids;
        index_item_ends = index_summary.item_end_chars;
        stats.index = SectionRenderStats {
            count: index_summary.count,
            chars: char_len(&output).saturating_sub(before),
        };
        stats.timings.push(crate::perf::PhaseTiming::elapsed(
            "section_index",
            section_start,
        ));
    }
    if !loaded.workstreams.is_empty() {
        let section_start = Instant::now();
        let before = char_len(&output);
        let workstream_summary = render_workstreams_with_summary(
            &mut output,
            &loaded.workstreams,
            policy.section_item_limit(SectionKind::Workstreams, 5),
            policy.section_char_limit(SectionKind::Workstreams, 1_200),
        );
        workstream_ids = workstream_summary.ids;
        workstream_item_ends = workstream_summary.item_end_chars;
        stats.workstreams = SectionRenderStats {
            count: workstream_summary.count,
            chars: char_len(&output).saturating_sub(before),
        };
        stats.timings.push(crate::perf::PhaseTiming::elapsed(
            "section_workstreams",
            section_start,
        ));
    }
    if !governed.summaries.is_empty() {
        let section_start = Instant::now();
        let before = char_len(&output);
        let session_item_limit =
            policy.section_item_limit(SectionKind::Sessions, policy.limits.session_limit);
        let session_summaries =
            &governed.summaries[..governed.summaries.len().min(session_item_limit)];
        let session_summary = render_recent_sessions_with_summary(
            &mut output,
            session_summaries,
            policy.section_char_limit(SectionKind::Sessions, 2_200),
        );
        session_ids = session_summary.ids;
        session_item_ends = session_summary.item_end_chars;
        stats.sessions = SectionRenderStats {
            count: session_summary.count,
            chars: char_len(&output).saturating_sub(before),
        };
        stats.timings.push(crate::perf::PhaseTiming::elapsed(
            "section_sessions",
            section_start,
        ));
    }
    let pre_total_governed_count = lesson_ids.len() + index_ids.len() + session_ids.len();
    stats.relevance.final_injected = pre_total_governed_count;
    stats.relevance.section_limited = relevance_plan
        .selected_count
        .saturating_sub(pre_total_governed_count);

    if debug {
        let diagnostics_start = Instant::now();
        super::diagnostics::apply_preference_diagnostics(
            conn,
            &request.project,
            preference_details.rendered_ids.clone(),
            &mut loaded.diagnostics,
        );
        output.push_str(&build_context_debug_trace(
            request, &policy, &loaded, &stats,
        ));
        stats.timings.push(crate::perf::PhaseTiming::elapsed(
            "debug_trace",
            diagnostics_start,
        ));
    }

    let core_item_ends = core_item_ends
        .iter()
        .map(|end| core_output_start + end)
        .collect::<Vec<_>>();
    let core_selected_ids = stats.core_ids.clone();
    let bounds = RenderedIdentityBounds {
        preference_ids: &preference_details.rendered_ids,
        preference_ends: &preference_item_ends,
        core_ids: &core_selected_ids,
        core_ends: &core_item_ends,
        lesson_ids: &lesson_ids,
        lesson_ends: &lesson_item_ends,
        index_ids: &index_ids,
        index_ends: &index_item_ends,
        session_ids: &session_ids,
        session_ends: &session_item_ends,
        workstream_ids: &workstream_ids,
        workstream_ends: &workstream_item_ends,
    };
    let finalized = finalize_context_output(
        std::mem::take(&mut output),
        &mut stats,
        request.use_colors,
        &bounds,
    )?;
    output = finalized.output;
    if let Some(bundle) = &mut context_bundle {
        super::render_bundle::seal_after_render(
            bundle,
            &finalized.final_preference_ids,
            &finalized.final_core_ids,
            &finalized.final_lesson_ids,
            &finalized.final_index_ids,
            &finalized.final_session_ids,
            &finalized.final_workstream_ids,
            &finalized.total_truncated_keys,
            &output,
        )?;
    }
    let audit_start = Instant::now();
    let audit_render = ContextAuditRenderState {
        core_selected_ids: &core_selected_ids,
        core_final_ids: &finalized.final_core_ids,
        index_final_ids: &finalized.final_index_ids,
        lesson_final_ids: &finalized.final_lesson_ids,
        session_final_ids: &finalized.final_session_ids,
        workstream_selected_ids: &workstream_ids,
        workstream_final_ids: &finalized.final_workstream_ids,
        item_end_chars: &finalized.item_end_chars,
    };
    let audit_items = build_context_audit_items(
        &loaded,
        &audit_render,
        &relevance_plan,
        &finalized.total_truncated_keys,
    );
    stats.timings.push(crate::perf::PhaseTiming::elapsed(
        "audit_items",
        audit_start,
    ));
    stats.timings.push(crate::perf::PhaseTiming::elapsed(
        "render_total",
        render_total_start,
    ));
    Ok(RenderedContext {
        output,
        stats,
        audit_items,
        data_version,
        has_load_errors,
        #[cfg(test)]
        context_bundle,
    })
}

fn open_context_connection_or_error(
    request: &ContextRequest,
    policy: &ContextPolicy,
) -> std::result::Result<rusqlite::Connection, Box<RenderedContext>> {
    match db::open_db_no_migrate() {
        Ok(conn) => Ok(conn),
        Err(error) => {
            crate::log::error(
                "context",
                &format!("db open failed for project={}: {}", request.project, error),
            );
            Err(Box::new(render_context_open_error(request, policy, error)))
        }
    }
}

fn render_context_open_error(
    request: &ContextRequest,
    policy: &ContextPolicy,
    error: anyhow::Error,
) -> RenderedContext {
    let mut output = super::render_error::context_error_output(
        request,
        &[ContextLoadError::new(
            "database",
            format!("failed to open remem database: {error}"),
        )],
    );
    let mut stats = stats::empty_stats(request);
    stats.total_char_limit = policy.limits.total_char_limit;
    stats.output_chars = char_len(&output);
    enforce_total_char_limit_preserving_footer(&mut output, policy.limits.total_char_limit, "");
    RenderedContext {
        output,
        stats,
        audit_items: Vec::new(),
        data_version: None,
        has_load_errors: true,
        #[cfg(test)]
        context_bundle: None,
    }
}

pub(in crate::context) fn empty_context_output(request: &ContextRequest) -> String {
    let header = build_context_header_with_style(
        &request.project,
        request.current_branch.as_deref(),
        request.hook_source.as_deref(),
        request.host,
        request.use_colors,
    );
    empty_state_output(&header, context_source_note(request.hook_source.as_deref()))
}
