use std::time::Instant;

use anyhow::Result;

use crate::db;

use super::audit::{build_context_audit_items, record_context_injection_items, ContextAuditItem};
use super::format::{char_len, truncate_chars_with_ellipsis};
use super::host::resolve_profile;
use super::injection_gate::{
    apply_context_gate_with_data_version, compute_data_version_hint, pre_render_context_gate,
    ContextGateAction, ContextGateDecision, ContextGatePrecheck,
};
use super::invocation::{
    direct_context_invocation, resolve_context_invocation, ContextCliOptions, ContextInvocation,
};
use super::policy::{ContextPolicy, SectionKind};
use super::render_inputs::{load_context_render_inputs, ContextRenderInputs};
use super::sections::{
    empty_state_output, render_core_memory_with_limits_and_staleness,
    render_lessons_with_summary_and_staleness, render_memory_index_with_summary_and_staleness,
    render_recent_sessions_with_limit, render_workstreams_with_summary,
};
use super::types::{ContextLoadError, ContextRequest};
mod eval;
mod stats;
mod timer;
pub(crate) use eval::{governance_eval_snapshot, session_start_eval_snapshot};
pub(in crate::context) use stats::{ContextRenderStats, SectionRenderStats};
use timer::log_context_timer;

pub(in crate::context) use super::debug::build_context_debug_trace;

pub(crate) const RENDER_CONTRACT_VERSION: u32 = 1;

pub(in crate::context) struct RenderedContext {
    pub(in crate::context) output: String,
    pub(in crate::context) stats: ContextRenderStats,
    pub(in crate::context) audit_items: Vec<ContextAuditItem>,
    pub(in crate::context) data_version: Option<String>,
    pub(in crate::context) has_load_errors: bool,
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

fn generate_context_for_invocation(invocation: ContextInvocation, use_gate: bool) -> Result<()> {
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
    let db_open_start = Instant::now();
    let conn = match open_context_connection_or_error(&request, &policy) {
        Ok(conn) => conn,
        Err(rendered) => {
            let rendered = *rendered;
            let mut decision = if use_gate {
                ContextGateDecision {
                    output: rendered.output,
                    action: ContextGateAction::FailOpen,
                    reason: "db_open",
                    key: None,
                    context_hash: None,
                    output_mode: None,
                }
            } else {
                ContextGateDecision {
                    output: rendered.output,
                    action: ContextGateAction::Bypassed,
                    reason: "legacy_direct",
                    key: None,
                    context_hash: None,
                    output_mode: None,
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
            print!(
                "{}",
                context_stdout_for_invocation(&decision.output, &invocation)?
            );
            log_context_timer(
                timer,
                &request,
                &decision,
                &rendered.stats,
                ContextGatePrecheck::Off,
            );
            return Ok(());
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
            let decision = if has_load_errors {
                ContextGateDecision {
                    output,
                    action: ContextGateAction::FailOpen,
                    reason: "context_load_errors",
                    key: None,
                    context_hash: None,
                    output_mode: Some("fail_open"),
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
    print!(
        "{}",
        context_stdout_for_invocation(&decision.output, &invocation)?
    );
    log_context_timer(timer, &request, &decision, &stats, precheck);
    Ok(())
}

#[cfg(test)]
pub(in crate::context) fn generate_context_for_test(
    invocation: ContextInvocation,
    use_gate: bool,
) -> Result<()> {
    generate_context_for_invocation(invocation, use_gate)
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
    )
}

fn render_context_output_from_inputs(
    conn: &rusqlite::Connection,
    request: &ContextRequest,
    invocation: Option<&ContextInvocation>,
    debug: bool,
    policy: ContextPolicy,
    inputs: ContextRenderInputs,
    prechecked_data_version: Option<String>,
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

    if preference_summary.rendered == 0
        && loaded.memories.is_empty()
        && loaded.lessons.is_empty()
        && loaded.summaries.is_empty()
        && loaded.workstreams.is_empty()
        && loaded.errors.is_empty()
    {
        return Ok(RenderedContext {
            output: empty_context_output(request),
            stats: empty_stats(request),
            audit_items: Vec::new(),
            data_version,
            has_load_errors,
        });
    }

    let mut output = String::new();
    let (mut lesson_ids, mut index_ids, mut workstream_ids) = (Vec::new(), Vec::new(), Vec::new());
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
    stats.timings.push(load_timing);
    stats.timings.push(preference_timing);

    let section_start = Instant::now();
    let before = char_len(&output);
    output.push_str(&preference_output);
    stats.preferences = SectionRenderStats {
        count: preference_summary.rendered,
        chars: char_len(&output).saturating_sub(before),
    };
    stats.timings.push(crate::perf::PhaseTiming::elapsed(
        "section_preferences",
        section_start,
    ));

    if !loaded.lessons.is_empty() {
        let section_start = Instant::now();
        let before = char_len(&output);
        let lesson_summary = render_lessons_with_summary_and_staleness(
            &mut output,
            &loaded.lessons,
            policy.section_item_limit(SectionKind::Lessons, policy.limits.lesson_limit),
            policy.section_char_limit(SectionKind::Lessons, policy.limits.lesson_char_limit),
            loaded.render_reference_epoch,
            &loaded.staleness_labels,
        );
        lesson_ids = lesson_summary.ids;
        stats.lessons = SectionRenderStats {
            count: lesson_summary.count,
            chars: char_len(&output).saturating_sub(before),
        };
        stats.timings.push(crate::perf::PhaseTiming::elapsed(
            "section_lessons",
            section_start,
        ));
    }
    if !loaded.memories.is_empty() {
        let render_limits = section_render_limits(&policy);
        let section_start = Instant::now();
        let before = char_len(&output);
        let core_summary = render_core_memory_with_limits_and_staleness(
            &mut output,
            &loaded.memories,
            &render_limits,
            loaded.render_reference_epoch,
            &loaded.staleness_labels,
        );
        let core_count = core_summary.count;
        stats.core_ids = core_summary.ids.clone();
        stats.core = SectionRenderStats {
            count: core_count,
            chars: char_len(&output).saturating_sub(before),
        };
        stats.timings.push(crate::perf::PhaseTiming::elapsed(
            "section_core",
            section_start,
        ));
        let section_start = Instant::now();
        let before = char_len(&output);
        let core_ids = core_summary.ids.into_iter().collect();
        let index_summary = render_memory_index_with_summary_and_staleness(
            &mut output,
            &loaded.memories,
            &render_limits,
            &core_ids,
            loaded.render_reference_epoch,
            &loaded.staleness_labels,
        );
        index_ids = index_summary.ids;
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
        stats.workstreams = SectionRenderStats {
            count: workstream_summary.count,
            chars: char_len(&output).saturating_sub(before),
        };
        stats.timings.push(crate::perf::PhaseTiming::elapsed(
            "section_workstreams",
            section_start,
        ));
    }
    if !loaded.summaries.is_empty() {
        let section_start = Instant::now();
        let before = char_len(&output);
        let session_count = render_recent_sessions_with_limit(
            &mut output,
            &loaded.summaries,
            policy.section_char_limit(SectionKind::Sessions, 2_200),
        );
        stats.sessions = SectionRenderStats {
            count: session_count,
            chars: char_len(&output).saturating_sub(before),
        };
        stats.timings.push(crate::perf::PhaseTiming::elapsed(
            "section_sessions",
            section_start,
        ));
    }

    if debug {
        let diagnostics_start = Instant::now();
        super::diagnostics::apply_preference_diagnostics(
            conn,
            &request.project,
            preference_details.rendered_ids,
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

    stats.output_chars = char_len(&output);
    let mut stats_footer = build_context_stats_footer_with_style(&stats, request.use_colors);
    stats.output_chars += char_len(&stats_footer);
    stats.truncated = stats.total_char_limit > 0 && stats.output_chars > stats.total_char_limit;
    stats_footer = build_context_stats_footer_with_style(&stats, request.use_colors);
    stats.output_chars = char_len(&output) + char_len(&stats_footer);
    output.push_str(&stats_footer);
    enforce_total_char_limit_preserving_footer(
        &mut output,
        policy.limits.total_char_limit,
        &stats_footer,
    );
    let audit_start = Instant::now();
    let audit_items = build_context_audit_items(
        &loaded,
        &stats.core_ids,
        &index_ids,
        &lesson_ids,
        &workstream_ids,
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
    let mut output = context_error_output(
        request,
        &[ContextLoadError::new(
            "database",
            format!("failed to open remem database: {error}"),
        )],
    );
    let mut stats = empty_stats(request);
    stats.total_char_limit = policy.limits.total_char_limit;
    stats.output_chars = char_len(&output);
    enforce_total_char_limit_preserving_footer(&mut output, policy.limits.total_char_limit, "");
    RenderedContext {
        output,
        stats,
        audit_items: Vec::new(),
        data_version: None,
        has_load_errors: true,
    }
}

fn context_error_output(request: &ContextRequest, errors: &[ContextLoadError]) -> String {
    let mut output = String::new();
    output.push_str(&build_context_header_with_style(
        &request.project,
        request.current_branch.as_deref(),
        request.hook_source.as_deref(),
        request.host,
        request.use_colors,
    ));
    if let Some(note) = context_source_note(request.hook_source.as_deref()) {
        output.push_str(note);
        output.push('\n');
    }
    output.push('\n');
    render_context_load_errors(&mut output, errors);
    output
}

fn render_context_load_errors(output: &mut String, errors: &[ContextLoadError]) {
    if errors.is_empty() {
        return;
    }

    output.push_str("## Context Load Errors\n");
    for error in errors {
        output.push_str("- ");
        output.push_str(error.section);
        output.push_str(": ");
        output.push_str(&truncate_chars_with_ellipsis(
            &error.message.replace('\n', " "),
            240,
        ));
        output.push('\n');
    }
    output.push('\n');
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

fn empty_stats(request: &ContextRequest) -> ContextRenderStats {
    ContextRenderStats {
        host: request.host.as_env_value().to_string(),
        branch: request.current_branch.clone(),
        hook_source: request.hook_source.clone(),
        ..ContextRenderStats::default()
    }
}

fn section_render_limits(policy: &ContextPolicy) -> super::policy::ContextLimits {
    let mut limits = policy.limits;
    limits.core_item_limit = policy.section_item_limit(SectionKind::Core, limits.core_item_limit);
    limits.core_char_limit = policy.section_char_limit(SectionKind::Core, limits.core_char_limit);
    limits.memory_index_limit =
        policy.section_item_limit(SectionKind::MemoryIndex, limits.memory_index_limit);
    limits.memory_index_char_limit =
        policy.section_char_limit(SectionKind::MemoryIndex, limits.memory_index_char_limit);
    limits
}

#[cfg(test)]
pub(in crate::context) fn build_context_stats_footer(stats: &ContextRenderStats) -> String {
    build_context_stats_footer_with_style(stats, false)
}

fn build_context_stats_footer_with_style(stats: &ContextRenderStats, use_colors: bool) -> String {
    super::style::context_stats_footer(stats, use_colors)
}

fn context_source_note(source: Option<&str>) -> Option<&'static str> {
    match source?.trim().to_ascii_lowercase().as_str() {
        "compact" => Some("Codex compacted the chat, so remem refreshed memory context."),
        "clear" => Some("Context was reloaded after an explicit clear."),
        _ => None,
    }
}

fn context_debug_enabled() -> bool {
    std::env::var("REMEM_CONTEXT_DEBUG")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

#[cfg(test)]
pub(in crate::context) fn enforce_total_char_limit(output: &mut String, char_limit: usize) {
    enforce_total_char_limit_preserving_footer(output, char_limit, "");
}

pub(in crate::context) fn enforce_total_char_limit_preserving_footer(
    output: &mut String,
    char_limit: usize,
    footer: &str,
) {
    if char_limit == 0 || output.chars().count() <= char_limit {
        return;
    }

    let marker = "\n[remem context truncated to REMEM_CONTEXT_TOTAL_CHAR_LIMIT]\n";
    let marker_chars = marker.chars().count();
    let footer_chars = footer.chars().count();

    if !footer.is_empty() && output.ends_with(footer) && marker_chars + footer_chars <= char_limit {
        let keep_chars = char_limit - marker_chars - footer_chars;
        let body = output.strip_suffix(footer).unwrap_or(output.as_str());
        let mut truncated: String = body.chars().take(keep_chars).collect();
        truncated.push_str(marker);
        truncated.push_str(footer);
        *output = truncated;
        return;
    }

    if marker_chars >= char_limit {
        *output = output.chars().take(char_limit).collect();
        return;
    }

    let keep_chars = char_limit - marker_chars;
    let mut truncated: String = output.chars().take(keep_chars).collect();
    truncated.push_str(marker);
    *output = truncated;
}

fn build_context_header_with_style(
    project: &str,
    current_branch: Option<&str>,
    hook_source: Option<&str>,
    host: super::host::HostKind,
    use_colors: bool,
) -> String {
    super::style::context_header(project, current_branch, hook_source, host, use_colors)
}

pub(in crate::context) fn context_stdout_for_invocation(
    output: &str,
    invocation: &ContextInvocation,
) -> Result<String> {
    if output.is_empty() || !is_codex_session_start_hook(invocation) {
        return Ok(output.to_string());
    }

    let additional_context = super::style::strip_ansi(output);
    let hook_output = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": additional_context,
        }
    });
    Ok(format!("{}\n", serde_json::to_string(&hook_output)?))
}

fn is_codex_session_start_hook(invocation: &ContextInvocation) -> bool {
    if invocation.host != super::host::HostKind::CodexCli {
        return false;
    }

    matches!(
        invocation
            .source
            .as_deref()
            .map(|source| source.trim().to_ascii_lowercase()),
        Some(source)
            if matches!(source.as_str(), "startup" | "resume" | "clear" | "compact")
    )
}
