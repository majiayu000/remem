use anyhow::Result;

use crate::db;

use super::format::{char_len, format_header_datetime, truncate_chars_with_ellipsis};
use super::host::resolve_profile;
use super::injection_gate::{apply_context_gate, ContextGateAction, ContextGateDecision};
use super::invocation::{
    direct_context_invocation, resolve_context_invocation, ContextCliOptions, ContextInvocation,
};
use super::policy::{ContextPolicy, SectionKind};
use super::query::load_context_data_with_policy;
use super::sections::{
    empty_state_output, render_core_memory_with_limits, render_lessons_with_limit,
    render_memory_index_with_limits_excluding, render_recent_sessions_with_limit,
    render_workstreams_with_limits,
};
use super::types::ContextRequest;

struct RenderedContext {
    output: String,
    stats: ContextRenderStats,
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
    let request = ContextRequest {
        cwd: invocation.cwd.clone(),
        project: invocation.project.clone(),
        session_id: invocation.session_id.clone(),
        hook_source: invocation.source.clone(),
        current_branch: db::detect_git_branch(&invocation.cwd),
        host: invocation.host,
        use_colors: invocation.use_colors,
    };
    let rendered = render_context_output(&request, invocation.debug || context_debug_enabled())?;
    let decision = if use_gate {
        apply_context_gate(&invocation, rendered.output)
    } else {
        ContextGateDecision {
            output: rendered.output,
            action: ContextGateAction::Bypassed,
            reason: "legacy_direct",
        }
    };
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

fn render_context_output(request: &ContextRequest, debug: bool) -> Result<RenderedContext> {
    let profile = resolve_profile(request.host);
    let policy = profile.default_policy();
    let conn = match db::open_db() {
        Ok(connection) => connection,
        Err(error) => {
            crate::log::error(
                "context",
                &format!("open_db failed for project={}: {}", request.project, error),
            );
            return Ok(RenderedContext {
                output: empty_context_output(request),
                stats: empty_stats(request),
            });
        }
    };

    let loaded = load_context_data_with_policy(
        &conn,
        &request.project,
        request.current_branch.as_deref(),
        &policy,
    );
    let (preference_output, preference_summary) =
        match render_preferences_to_buffer(&conn, &request.project, &request.cwd, &policy) {
            Ok(rendered) => rendered,
            Err(error) => {
                crate::log::warn("context", &format!("render_preferences failed: {}", error));
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
    {
        return Ok(RenderedContext {
            output: empty_context_output(request),
            stats: empty_stats(request),
        });
    }

    let mut output = String::new();
    output.push_str(&build_context_header(
        &request.project,
        request.current_branch.as_deref(),
        request.hook_source.as_deref(),
    ));
    output.push_str(profile.retrieval_hints().line);
    output.push('\n');
    if let Some(note) = context_source_note(request.hook_source.as_deref()) {
        output.push_str(note);
        output.push('\n');
    }
    output.push('\n');

    let mut stats = ContextRenderStats {
        host: request.host.as_env_value().to_string(),
        branch: request.current_branch.clone(),
        hook_source: request.hook_source.clone(),
        total_char_limit: policy.limits.total_char_limit,
        memories_loaded: loaded.memories.len() + loaded.lessons.len(),
        project_preferences: preference_summary.project_rendered,
        global_preferences: preference_summary.global_rendered,
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
    let mut stats_footer = build_context_stats_footer(&stats);
    stats.output_chars += char_len(&stats_footer);
    stats.truncated = stats.total_char_limit > 0 && stats.output_chars > stats.total_char_limit;
    stats_footer = build_context_stats_footer(&stats);
    stats.output_chars = char_len(&output) + char_len(&stats_footer);
    output.push_str(&stats_footer);
    enforce_total_char_limit_preserving_footer(
        &mut output,
        policy.limits.total_char_limit,
        &stats_footer,
    );
    Ok(RenderedContext { output, stats })
}

pub(in crate::context) fn empty_context_output(request: &ContextRequest) -> String {
    let header = build_context_header(
        &request.project,
        request.current_branch.as_deref(),
        request.hook_source.as_deref(),
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
    pub core_ids: Vec<i64>,
    pub output_chars: usize,
    pub truncated: bool,
}

pub(in crate::context) fn build_context_stats_footer(stats: &ContextRenderStats) -> String {
    let branch = stats.branch.as_deref().unwrap_or("-");
    let source = context_source_footer(stats.hook_source.as_deref());
    let estimated_tokens = estimate_tokens(stats.output_chars);
    format!(
        "{} context memories loaded. {} core ({} chars). {} lessons ({} chars). {} indexed ({} chars). {} preferences (project:{} global:{}, {} chars). {} sessions ({} chars). host={} source={} branch={} total={} chars/~{} tokens limit={} truncated={}\n",
        stats.memories_loaded,
        stats.core.count,
        stats.core.chars,
        stats.lessons.count,
        stats.lessons.chars,
        stats.index.count,
        stats.index.chars,
        stats.preferences.count,
        stats.project_preferences,
        stats.global_preferences,
        stats.preferences.chars,
        stats.sessions.count,
        stats.sessions.chars,
        stats.host,
        source,
        branch,
        stats.output_chars,
        estimated_tokens,
        stats.total_char_limit,
        if stats.truncated { "yes" } else { "no" },
    )
}

fn build_context_debug_trace(
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
        "- preferences project_rendered={} global_rendered={} reason=scope_limits_then_claude_md_dedup\n",
        stats.project_preferences, stats.global_preferences
    ));
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
        "compact" => Some(
            "REMEM_CONTEXT_SOURCE=compact: Codex compact triggered this memory context reload.",
        ),
        "clear" => {
            Some("REMEM_CONTEXT_SOURCE=clear: context was reloaded after an explicit clear.")
        }
        _ => None,
    }
}

fn context_debug_enabled() -> bool {
    std::env::var("REMEM_CONTEXT_DEBUG")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn estimate_tokens(chars: usize) -> usize {
    (chars + 3) / 4
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

pub(in crate::context) fn build_context_header(
    project: &str,
    current_branch: Option<&str>,
    hook_source: Option<&str>,
) -> String {
    let branch_label = current_branch
        .map(|branch| format!(" @{}", branch))
        .unwrap_or_default();
    let source_label = context_source_header_label(hook_source)
        .map(|label| format!(" [{}]", label))
        .unwrap_or_default();
    format!(
        "# [{}{}] context {}{}\n",
        project,
        branch_label,
        format_header_datetime(),
        source_label
    )
}

fn context_source_header_label(source: Option<&str>) -> Option<&'static str> {
    match source?.trim().to_ascii_lowercase().as_str() {
        "compact" => Some("REMEM POST-COMPACT RELOAD"),
        "clear" => Some("REMEM CLEAR RELOAD"),
        _ => None,
    }
}

fn context_source_footer(source: Option<&str>) -> &'static str {
    match source
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("compact") => "compact",
        Some("clear") => "clear",
        _ => "-",
    }
}
