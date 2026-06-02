use std::collections::HashSet;

use anyhow::Result;

use crate::db;

use super::format::{char_len, truncate_chars_with_ellipsis};
use super::host::resolve_profile;
use super::injection_gate::{apply_context_gate, ContextGateAction, ContextGateDecision};
use super::invocation::{
    direct_context_invocation, resolve_context_invocation, ContextCliOptions, ContextInvocation,
};
use super::ownership::OwnerCounts;
use super::policy::{ContextLimits, ContextPolicy, SectionKind};
use super::query::load_context_data_with_policy;
use super::sections::{
    empty_state_output, render_core_memory_with_limits, render_lessons_with_limit,
    render_memory_index_with_limits_excluding, render_recent_sessions_with_limit,
    render_workstreams_with_limits,
};
use super::types::{ContextLoadError, ContextRequest};

pub(in crate::context) struct RenderedContext {
    pub(in crate::context) output: String,
    pub(in crate::context) stats: ContextRenderStats,
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
    let loaded = load_context_data_with_policy(conn, project, current_branch, &policy);
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
        render_lessons_with_limit(
            &mut output,
            &loaded.lessons,
            policy.section_item_limit(SectionKind::Lessons, policy.limits.lesson_limit),
            policy.section_char_limit(SectionKind::Lessons, policy.limits.lesson_char_limit),
        );
    }
    if !loaded.memories.is_empty() {
        let render_limits = section_render_limits(policy);
        let core_summary =
            render_core_memory_with_limits(&mut output, &loaded.memories, &render_limits);
        let core_ids: HashSet<i64> = core_summary.ids.into_iter().collect();
        render_memory_index_with_limits_excluding(
            &mut output,
            &loaded.memories,
            &render_limits,
            &core_ids,
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
    let rendered = render_context_output(&request, debug_enabled)?;
    let mut decision = if use_gate {
        apply_context_gate(&invocation, rendered.output)
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
        append_context_gate_debug_trace(&mut decision.output, &request, &decision_for_debug);
    }
    print!("{}", decision.output);

    let capabilities = resolve_profile(request.host).capabilities();
    timer.done(&format!(
        "project={} cwd={} session={} host={} colors={} gate={:?}:{} caps=[mcp:{} session_start:{} prompt_submit:{} native_edits:{} bash:{}] context_memories={} core={} lessons={} index={} preferences={} sessions={} workstreams={}",
        request.project,
        request.cwd,
        request.session_id.as_deref().unwrap_or("-"),
        request.host.as_env_value(),
        request.use_colors,
        decision.action,
        decision.reason,
        capabilities.has_mcp_tools,
        capabilities.has_session_start_hook,
        capabilities.has_user_prompt_submit_hook,
        capabilities.observes_native_file_edits,
        capabilities.observes_bash,
        rendered.stats.memories_loaded,
        rendered.stats.core.count,
        rendered.stats.lessons.count,
        rendered.stats.index.count,
        rendered.stats.preferences.count,
        rendered.stats.sessions.count,
        rendered.stats.workstreams.count,
    ));
    Ok(())
}

fn append_context_gate_debug_trace(
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
    if output.trim_start().starts_with("## Debug Trace") {
        output.push_str(&format!(
            "- request host={} project={} cwd={} branch={} session={} source={}\n",
            request.host.as_env_value(),
            request.project,
            request.cwd,
            request.current_branch.as_deref().unwrap_or("-"),
            request.session_id.as_deref().unwrap_or("-"),
            request.hook_source.as_deref().unwrap_or("-")
        ));
    }
    output.push_str(&format!(
        "- gate action={:?} reason={} output_mode={} key={} hash={}\n",
        decision.action,
        decision.reason,
        decision.output_mode.unwrap_or("-"),
        decision.key.as_deref().unwrap_or("-"),
        decision.context_hash.as_deref().unwrap_or("-")
    ));
}

pub(in crate::context) fn render_context_output(
    request: &ContextRequest,
    debug: bool,
) -> Result<RenderedContext> {
    let profile = resolve_profile(request.host);
    let policy = profile.default_policy();
    let conn = match db::open_db() {
        Ok(connection) => connection,
        Err(error) => {
            crate::log::error(
                "context",
                &format!("open_db failed for project={}: {}", request.project, error),
            );
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
            enforce_total_char_limit_preserving_footer(
                &mut output,
                policy.limits.total_char_limit,
                "",
            );
            return Ok(RenderedContext { output, stats });
        }
    };

    let mut loaded = load_context_data_with_policy(
        &conn,
        &request.project,
        request.current_branch.as_deref(),
        &policy,
    );
    let (preference_output, preference_summary) =
        match render_preferences_to_buffer(&conn, &request.project, &request.cwd, &policy) {
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
                    crate::memory::preference::PreferenceRenderSummary::default(),
                )
            }
        };

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
        });
    }

    let mut output = String::new();
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
        let lesson_count = render_lessons_with_limit(
            &mut output,
            &loaded.lessons,
            policy.section_item_limit(SectionKind::Lessons, policy.limits.lesson_limit),
            policy.section_char_limit(SectionKind::Lessons, policy.limits.lesson_char_limit),
        );
        stats.lessons = SectionRenderStats {
            count: lesson_count,
            chars: char_len(&output).saturating_sub(before),
        };
    }
    if !loaded.memories.is_empty() {
        let render_limits = section_render_limits(&policy);
        let before = char_len(&output);
        let core_summary =
            render_core_memory_with_limits(&mut output, &loaded.memories, &render_limits);
        let core_count = core_summary.count;
        stats.core_ids = core_summary.ids.clone();
        stats.core = SectionRenderStats {
            count: core_count,
            chars: char_len(&output).saturating_sub(before),
        };
        let before = char_len(&output);
        let core_ids = core_summary.ids.into_iter().collect();
        let index_count = render_memory_index_with_limits_excluding(
            &mut output,
            &loaded.memories,
            &render_limits,
            &core_ids,
        );
        stats.index = SectionRenderStats {
            count: index_count,
            chars: char_len(&output).saturating_sub(before),
        };
    }
    if !loaded.workstreams.is_empty() {
        let before = char_len(&output);
        let workstream_count = render_workstreams_with_limits(
            &mut output,
            &loaded.workstreams,
            policy.section_item_limit(SectionKind::Workstreams, 5),
            policy.section_char_limit(SectionKind::Workstreams, 1_200),
        );
        stats.workstreams = SectionRenderStats {
            count: workstream_count,
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
    Ok(RenderedContext { output, stats })
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
) -> Result<(String, crate::memory::preference::PreferenceRenderSummary)> {
    let mut output = String::new();
    let limits = &policy.limits;
    let summary = crate::memory::preference::render_preferences_with_limits_detailed(
        &mut output,
        conn,
        project,
        cwd,
        limits.preference_project_limit,
        limits.preference_global_limit,
        limits.preference_char_limit,
    )?;
    Ok((output, summary))
}

#[derive(Debug, Clone, Default)]
pub(in crate::context) struct SectionRenderStats {
    pub count: usize,
    pub chars: usize,
}

#[derive(Debug, Clone, Default)]
pub(in crate::context) struct ContextRenderStats {
    pub host: String,
    pub branch: Option<String>,
    pub hook_source: Option<String>,
    pub total_char_limit: usize,
    pub memories_loaded: usize,
    pub core: SectionRenderStats,
    pub lessons: SectionRenderStats,
    pub index: SectionRenderStats,
    pub preferences: SectionRenderStats,
    pub project_preferences: usize,
    pub global_preferences: usize,
    pub sessions: SectionRenderStats,
    pub workstreams: SectionRenderStats,
    pub owner_counts: OwnerCounts,
    pub core_ids: Vec<i64>,
    pub output_chars: usize,
    pub truncated: bool,
}

#[cfg(test)]
pub(in crate::context) fn build_context_stats_footer(stats: &ContextRenderStats) -> String {
    build_context_stats_footer_with_style(stats, false)
}

fn build_context_stats_footer_with_style(stats: &ContextRenderStats, use_colors: bool) -> String {
    super::style::context_stats_footer(stats, use_colors)
}

pub(in crate::context) fn build_context_debug_trace(
    request: &ContextRequest,
    policy: &ContextPolicy,
    loaded: &super::types::LoadedContext,
    stats: &ContextRenderStats,
) -> String {
    let mut trace = String::from("## Debug Trace\n");
    trace.push_str(&format!(
        "- request host={} project={} cwd={} branch={} session={} source={}\n",
        request.host.as_env_value(),
        request.project,
        request.cwd,
        request.current_branch.as_deref().unwrap_or("-"),
        request.session_id.as_deref().unwrap_or("-"),
        request.hook_source.as_deref().unwrap_or("-")
    ));
    trace.push_str(&format!(
        "- limits total={} core_items={} core_chars={} lessons={} lesson_chars={} index_items={} index_chars={} sessions={} preferences(project={}, global={}, chars={})\n",
        policy.limits.total_char_limit,
        policy.limits.core_item_limit,
        policy.limits.core_char_limit,
        policy.limits.lesson_limit,
        policy.limits.lesson_char_limit,
        policy.limits.memory_index_limit,
        policy.limits.memory_index_char_limit,
        policy.limits.session_limit,
        policy.limits.preference_project_limit,
        policy.limits.preference_global_limit,
        policy.limits.preference_char_limit
    ));
    trace.push_str(&format!(
        "- rendered core={} lessons={} index={} preferences={} sessions={} workstreams={}\n",
        stats.core.count,
        stats.lessons.count,
        stats.index.count,
        stats.preferences.count,
        stats.sessions.count,
        stats.workstreams.count
    ));
    trace.push_str(&format!(
        "- rendered_chars core={} lessons={} index={} preferences={} sessions={} workstreams={}\n",
        stats.core.chars,
        stats.lessons.chars,
        stats.index.chars,
        stats.preferences.chars,
        stats.sessions.chars,
        stats.workstreams.chars
    ));
    trace.push_str(&format!(
        "- stats host={} branch={} source={}\n",
        stats.host,
        stats.branch.as_deref().unwrap_or("-"),
        stats.hook_source.as_deref().unwrap_or("-")
    ));
    trace.push_str(&format!(
        "- preferences project_rendered={} global_rendered={} reason=scope_limits_then_claude_md_dedup\n",
        stats.project_preferences, stats.global_preferences
    ));
    trace.push_str(&format!(
        "- owner counts repo={} user={} workspace={} tool={} domain={} workstream={} session={} legacy={} unknown={}\n",
        stats.owner_counts.repo,
        stats.owner_counts.user,
        stats.owner_counts.workspace,
        stats.owner_counts.tool,
        stats.owner_counts.domain,
        stats.owner_counts.workstream,
        stats.owner_counts.session,
        stats.owner_counts.legacy,
        stats.owner_counts.unknown
    ));
    for trace_row in loaded.owner_traces.iter().take(60) {
        trace.push_str(&format!(
            "- owner {} id={} scope={} key={} source_project={} target_project={} domain={} context_class={} {} reason={} title={}\n",
            trace_row.object_kind,
            trace_row.id,
            trace_row.owner_scope.as_deref().unwrap_or("-"),
            trace_row.owner_key.as_deref().unwrap_or("-"),
            trace_row.source_project.as_deref().unwrap_or("-"),
            trace_row.target_project.as_deref().unwrap_or("-"),
            trace_row.topic_domain.as_deref().unwrap_or("-"),
            trace_row.context_class.as_deref().unwrap_or("-"),
            if trace_row.included {
                "included"
            } else {
                "excluded"
            },
            trace_row.reason,
            truncate_chars_with_ellipsis(&trace_row.title, 120)
        ));
    }
    if loaded.owner_traces.len() > 60 {
        trace.push_str(&format!(
            "- owner trace truncated: {} additional rows omitted\n",
            loaded.owner_traces.len() - 60
        ));
    }
    for (rank, lesson) in loaded.lessons.iter().enumerate() {
        trace.push_str(&format!(
            "- lesson rank={} id={} confidence={:.2} reinforced={} scope={} topic={} title={}\n",
            rank + 1,
            lesson.memory.id,
            lesson.metadata.confidence,
            lesson.metadata.reinforcement_count,
            lesson.memory.scope,
            lesson.memory.topic_key.as_deref().unwrap_or("-"),
            truncate_chars_with_ellipsis(&lesson.memory.title, 120)
        ));
    }
    for (rank, memory) in loaded.memories.iter().take(30).enumerate() {
        let target = if stats.core_ids.contains(&memory.id) {
            "core"
        } else {
            "index_candidate"
        };
        trace.push_str(&format!(
            "- memory rank={} id={} type={} scope={} branch={} topic={} target={} reason=loaded_after_scope_branch_dedupe title={}\n",
            rank + 1,
            memory.id,
            memory.memory_type,
            memory.scope,
            memory.branch.as_deref().unwrap_or("-"),
            memory.topic_key.as_deref().unwrap_or("-"),
            target,
            truncate_chars_with_ellipsis(&memory.title, 120)
        ));
    }
    if loaded.memories.len() > 30 {
        trace.push_str(&format!(
            "- memory trace truncated: {} additional candidates omitted\n",
            loaded.memories.len() - 30
        ));
    }
    for (rank, summary) in loaded
        .summaries
        .iter()
        .take(policy.limits.session_limit)
        .enumerate()
    {
        trace.push_str(&format!(
            "- session rank={} reason=recent_after_cluster_filter request={}\n",
            rank + 1,
            truncate_chars_with_ellipsis(&summary.request, 120)
        ));
    }
    trace.push('\n');
    trace
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

#[cfg(test)]
pub(in crate::context) fn build_context_header(
    project: &str,
    current_branch: Option<&str>,
    hook_source: Option<&str>,
) -> String {
    build_context_header_with_style(
        project,
        current_branch,
        hook_source,
        super::host::HostKind::Unknown,
        false,
    )
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
