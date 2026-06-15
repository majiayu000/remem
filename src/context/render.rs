use std::collections::HashSet;

use anyhow::Result;

use crate::db;

use super::audit::{build_context_audit_items, record_context_injection_items, ContextAuditItem};
use super::format::{char_len, truncate_chars_with_ellipsis};
use super::host::resolve_profile;
use super::injection_gate::{
    apply_context_gate_with_data_version, pre_render_context_gate, ContextGateAction,
    ContextGateDecision, ContextGatePrecheck,
};
use super::invocation::{
    direct_context_invocation, resolve_context_invocation, ContextCliOptions, ContextInvocation,
};
use super::policy::{ContextLimits, ContextPolicy, SectionKind};
use super::query::load_context_data_with_policy;
use super::sections::{
    empty_state_output, render_core_memory_with_limits_and_staleness,
    render_lessons_with_limit_and_staleness, render_lessons_with_summary_and_staleness,
    render_memory_index_with_limits_excluding_and_staleness,
    render_memory_index_with_summary_and_staleness, render_recent_sessions_with_limit,
    render_workstreams_with_limits, render_workstreams_with_summary,
};
use super::types::{ContextLoadError, ContextRequest};
mod stats;
mod timer;
pub(in crate::context) use stats::{ContextRenderStats, SectionRenderStats};
use timer::log_context_timer;

pub(in crate::context) use super::debug::build_context_debug_trace;

pub(in crate::context) struct RenderedContext {
    pub(in crate::context) output: String,
    pub(in crate::context) stats: ContextRenderStats,
    pub(in crate::context) audit_items: Vec<ContextAuditItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextEvalSnapshot {
    pub memory_topic_keys: Vec<String>,
    pub memory_titles: Vec<String>,
    pub rendered_output: String,
    pub total_included: usize,
    pub safe_owner_included: usize,
    pub unsafe_owner_included: usize,
    pub excluded_owner_titles: Vec<String>,
}

pub(crate) fn governance_eval_snapshot(
    conn: &rusqlite::Connection,
    project: &str,
    current_branch: Option<&str>,
) -> Result<ContextEvalSnapshot> {
    let policy = ContextPolicy::from_limits(ContextLimits::default());
    let loaded = load_context_data_with_policy(conn, project, current_branch, &policy, true);
    let rendered_output = render_loaded_context_for_eval(conn, project, &policy, &loaded)?;
    let rendered_memories = loaded
        .memories
        .iter()
        .filter(|memory| rendered_output.contains(&memory.title))
        .collect::<Vec<_>>();
    let memory_topic_keys = rendered_memories
        .iter()
        .filter_map(|memory| memory.topic_key.clone())
        .collect::<Vec<_>>();
    let memory_titles = rendered_memories
        .iter()
        .map(|memory| memory.title.clone())
        .collect::<Vec<_>>();
    let unsafe_owner_included = loaded
        .owner_traces
        .iter()
        .filter(|trace| trace.included)
        .filter(|trace| !matches!(trace.owner_scope.as_deref(), Some("repo") | None))
        .count();
    let excluded_owner_titles = loaded
        .owner_traces
        .iter()
        .filter(|trace| !trace.included)
        .map(|trace| trace.title.clone())
        .collect::<Vec<_>>();
    let total_included = loaded.memories.len();

    Ok(ContextEvalSnapshot {
        memory_topic_keys,
        memory_titles,
        rendered_output,
        total_included,
        safe_owner_included: total_included.saturating_sub(unsafe_owner_included),
        unsafe_owner_included,
        excluded_owner_titles,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionStartEvalSnapshot {
    pub rendered_output: String,
    pub output_chars: usize,
    pub memories_loaded: usize,
    pub core_count: usize,
    pub index_count: usize,
    pub lesson_count: usize,
    pub preference_count: usize,
    pub session_count: usize,
    pub workstream_count: usize,
    pub truncated: bool,
}

pub(crate) fn session_start_eval_snapshot(
    cwd: &str,
    project: &str,
    current_branch: Option<&str>,
    host: &str,
) -> Result<SessionStartEvalSnapshot> {
    let request = ContextRequest {
        cwd: cwd.to_string(),
        project: project.to_string(),
        session_id: Some("eval-session-start".to_string()),
        hook_source: Some("SessionStart".to_string()),
        current_branch: current_branch.map(str::to_string),
        host: super::host::resolve_host_kind(Some(host)),
        use_colors: false,
    };
    let policy = ContextPolicy::from_limits(ContextLimits::default());
    let rendered = match open_context_connection_or_error(&request, &policy) {
        Ok(conn) => render_context_output_with_policy(&conn, &request, false, policy)?,
        Err(rendered) => *rendered,
    };
    Ok(SessionStartEvalSnapshot {
        rendered_output: rendered.output,
        output_chars: rendered.stats.output_chars,
        memories_loaded: rendered.stats.memories_loaded,
        core_count: rendered.stats.core.count,
        index_count: rendered.stats.index.count,
        lesson_count: rendered.stats.lessons.count,
        preference_count: rendered.stats.preferences.count,
        session_count: rendered.stats.sessions.count,
        workstream_count: rendered.stats.workstreams.count,
        truncated: rendered.stats.truncated,
    })
}

fn render_loaded_context_for_eval(
    conn: &rusqlite::Connection,
    project: &str,
    policy: &ContextPolicy,
    loaded: &super::types::LoadedContext,
) -> Result<String> {
    let (preference_output, _) = render_preferences_to_buffer(conn, project, project, policy)?;
    let mut output = preference_output;

    render_context_load_errors(&mut output, &loaded.errors);
    if !loaded.lessons.is_empty() {
        render_lessons_with_limit_and_staleness(
            &mut output,
            &loaded.lessons,
            policy.section_item_limit(SectionKind::Lessons, policy.limits.lesson_limit),
            policy.section_char_limit(SectionKind::Lessons, policy.limits.lesson_char_limit),
            &loaded.staleness_labels,
        );
    }
    if !loaded.memories.is_empty() {
        let render_limits = section_render_limits(policy);
        let core_summary = render_core_memory_with_limits_and_staleness(
            &mut output,
            &loaded.memories,
            &render_limits,
            &loaded.staleness_labels,
        );
        let core_ids: HashSet<i64> = core_summary.ids.into_iter().collect();
        render_memory_index_with_limits_excluding_and_staleness(
            &mut output,
            &loaded.memories,
            &render_limits,
            &core_ids,
            &loaded.staleness_labels,
        );
    }
    if !loaded.workstreams.is_empty() {
        render_workstreams_with_limits(
            &mut output,
            &loaded.workstreams,
            policy.section_item_limit(SectionKind::Workstreams, 5),
            policy.section_char_limit(SectionKind::Workstreams, 1_200),
        );
    }
    if !loaded.summaries.is_empty() {
        render_recent_sessions_with_limit(
            &mut output,
            &loaded.summaries,
            policy.section_char_limit(SectionKind::Sessions, 2_200),
        );
    }
    Ok(output)
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
            print!("{}", decision.output);
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
    let (mut decision, stats, precheck, audit_items) = if use_gate {
        let precheck =
            pre_render_context_gate(&conn, &invocation, &request, &policy, debug_enabled);
        if let Some(decision) = precheck.decision {
            (
                decision,
                ContextRenderStats::default(),
                precheck.precheck,
                Vec::new(),
            )
        } else {
            let rendered =
                render_context_output_with_policy(&conn, &request, debug_enabled, policy)?;
            let decision = apply_context_gate_with_data_version(
                &conn,
                &invocation,
                rendered.output,
                precheck.data_version.as_deref(),
            );
            (
                decision,
                rendered.stats,
                precheck.precheck,
                rendered.audit_items,
            )
        }
    } else {
        let rendered = render_context_output_with_policy(&conn, &request, debug_enabled, policy)?;
        (
            ContextGateDecision {
                output: rendered.output,
                action: ContextGateAction::Bypassed,
                reason: "legacy_direct",
                key: None,
                context_hash: None,
                output_mode: None,
            },
            rendered.stats,
            ContextGatePrecheck::Off,
            rendered.audit_items,
        )
    };
    if debug_enabled {
        let decision_for_debug = decision.clone();
        append_context_gate_debug_trace(&mut decision.output, &request, &decision_for_debug);
    }
    if !audit_items.is_empty() {
        if let Err(error) =
            record_context_injection_items(&conn, &invocation, &decision, &audit_items)
        {
            crate::log::warn(
                "context-audit",
                &format!("failed to write audit rows: {error}"),
            );
        }
    }
    print!("{}", decision.output);
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
    render_context_output_with_policy(&conn, request, debug, policy)
}

fn render_context_output_with_policy(
    conn: &rusqlite::Connection,
    request: &ContextRequest,
    debug: bool,
    policy: ContextPolicy,
) -> Result<RenderedContext> {
    let profile = resolve_profile(request.host);
    let mut loaded = load_context_data_with_policy(
        conn,
        &request.project,
        request.current_branch.as_deref(),
        &policy,
        debug,
    );
    let (preference_output, preference_details) =
        match render_preferences_to_buffer(conn, &request.project, &request.cwd, &policy) {
            Ok(rendered) => rendered,
            Err(error) => {
                let message = format!(
                    "failed to render preferences for {}: {error}",
                    request.project
                );
                crate::log::error("context", &message);
                loaded
                    .errors
                    .push(ContextLoadError::new("preferences", message));
                (
                    String::new(),
                    crate::memory::preference::PreferenceRenderDetails::default(),
                )
            }
        };
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

    let before = char_len(&output);
    output.push_str(&preference_output);
    stats.preferences = SectionRenderStats {
        count: preference_summary.rendered,
        chars: char_len(&output).saturating_sub(before),
    };

    if !loaded.lessons.is_empty() {
        let before = char_len(&output);
        let lesson_summary = render_lessons_with_summary_and_staleness(
            &mut output,
            &loaded.lessons,
            policy.section_item_limit(SectionKind::Lessons, policy.limits.lesson_limit),
            policy.section_char_limit(SectionKind::Lessons, policy.limits.lesson_char_limit),
            &loaded.staleness_labels,
        );
        lesson_ids = lesson_summary.ids;
        stats.lessons = SectionRenderStats {
            count: lesson_summary.count,
            chars: char_len(&output).saturating_sub(before),
        };
    }
    if !loaded.memories.is_empty() {
        let render_limits = section_render_limits(&policy);
        let before = char_len(&output);
        let core_summary = render_core_memory_with_limits_and_staleness(
            &mut output,
            &loaded.memories,
            &render_limits,
            &loaded.staleness_labels,
        );
        let core_count = core_summary.count;
        stats.core_ids = core_summary.ids.clone();
        stats.core = SectionRenderStats {
            count: core_count,
            chars: char_len(&output).saturating_sub(before),
        };
        let before = char_len(&output);
        let core_ids = core_summary.ids.into_iter().collect();
        let index_summary = render_memory_index_with_summary_and_staleness(
            &mut output,
            &loaded.memories,
            &render_limits,
            &core_ids,
            &loaded.staleness_labels,
        );
        index_ids = index_summary.ids;
        stats.index = SectionRenderStats {
            count: index_summary.count,
            chars: char_len(&output).saturating_sub(before),
        };
    }
    if !loaded.workstreams.is_empty() {
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
    }
    if !loaded.summaries.is_empty() {
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
    }

    if debug {
        super::diagnostics::apply_preference_diagnostics(
            conn,
            &request.project,
            preference_details.rendered_ids,
            &mut loaded.diagnostics,
        );
        output.push_str(&build_context_debug_trace(
            request, &policy, &loaded, &stats,
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
    let audit_items = build_context_audit_items(
        &loaded,
        &stats.core_ids,
        &index_ids,
        &lesson_ids,
        &workstream_ids,
    );
    Ok(RenderedContext {
        output,
        stats,
        audit_items,
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

fn render_preferences_to_buffer(
    conn: &rusqlite::Connection,
    project: &str,
    cwd: &str,
    policy: &ContextPolicy,
) -> Result<(String, crate::memory::preference::PreferenceRenderDetails)> {
    let mut output = String::new();
    let limits = &policy.limits;
    let details = crate::memory::preference::render_preferences_with_context_details(
        &mut output,
        conn,
        project,
        cwd,
        limits.preference_project_limit,
        limits.preference_global_limit,
        limits.preference_char_limit,
    )?;
    Ok((output, details))
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
