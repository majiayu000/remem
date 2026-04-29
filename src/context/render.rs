use anyhow::Result;

use crate::db;
use crate::db::project_from_cwd;

use super::format::format_header_datetime;
use super::host::{resolve_host_kind, resolve_profile};
use super::policy::{ContextPolicy, SectionKind};
use super::query::load_context_data_with_policy;
use super::sections::{
    render_core_memory_with_limits, render_empty_state, render_memory_index_with_limits,
    render_recent_sessions, render_workstreams_with_limits,
};
use super::types::ContextRequest;

pub fn generate_context(
    cwd: &str,
    session_id: Option<&str>,
    use_colors: bool,
    host_arg: Option<&str>,
) -> Result<()> {
    let timer = crate::log::Timer::start("context", &format!("cwd={}", cwd));
    let project = project_from_cwd(cwd);
    let current_branch = db::detect_git_branch(cwd);
    let host = resolve_host_kind(host_arg);
    let profile = resolve_profile(host);
    let policy = profile.default_policy();
    let request = ContextRequest {
        cwd: cwd.to_string(),
        project: project.clone(),
        session_id: session_id.map(str::to_string),
        current_branch: current_branch.clone(),
        host: profile.host(),
        use_colors,
    };
    let capabilities = profile.capabilities();

    let conn = match db::open_db() {
        Ok(connection) => connection,
        Err(error) => {
            crate::log::warn(
                "context",
                &format!("open_db failed for project={}: {}", project, error),
            );
            render_empty_state(&project);
            timer.done("empty (no DB)");
            return Ok(());
        }
    };

    let loaded = load_context_data_with_policy(
        &conn,
        &request.project,
        request.current_branch.as_deref(),
        &policy,
    );
    let (preference_output, preference_count) =
        match render_preferences_to_buffer(&conn, &project, cwd, &policy) {
            Ok(rendered) => rendered,
            Err(error) => {
                crate::log::warn("context", &format!("render_preferences failed: {}", error));
                (String::new(), 0)
            }
        };

    if preference_count == 0
        && loaded.memories.is_empty()
        && loaded.summaries.is_empty()
        && loaded.workstreams.is_empty()
    {
        render_empty_state(&project);
        timer.done("empty (no data)");
        return Ok(());
    }

    let mut output = String::new();
    output.push_str(&build_context_header(
        &request.project,
        request.current_branch.as_deref(),
    ));
    output.push_str(profile.retrieval_hints().line);
    output.push_str("\n\n");

    output.push_str(&preference_output);

    let mut core_count = 0;
    if !loaded.memories.is_empty() {
        let render_limits = section_render_limits(&policy);
        core_count = render_core_memory_with_limits(&mut output, &loaded.memories, &render_limits);
        render_memory_index_with_limits(&mut output, &loaded.memories, &render_limits);
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
        render_recent_sessions(&mut output, &loaded.summaries);
    }

    output.push_str(&format!(
        "{} indexed memories loaded. {} core memories. {} preferences. {} sessions.\n",
        loaded.memories.len(),
        core_count,
        preference_count,
        loaded.summaries.len()
    ));
    enforce_total_char_limit(&mut output, policy.limits.total_char_limit);
    print!("{}", output);

    timer.done(&format!(
        "project={} cwd={} session={} host={} colors={} caps=[mcp:{} session_start:{} prompt_submit:{} native_edits:{} bash:{}] indexed_memories={} core={} preferences={} summaries={} workstreams={}",
        request.project,
        request.cwd,
        request.session_id.as_deref().unwrap_or("-"),
        request.host.as_env_value(),
        request.use_colors,
        capabilities.has_mcp_tools,
        capabilities.has_session_start_hook,
        capabilities.has_user_prompt_submit_hook,
        capabilities.observes_native_file_edits,
        capabilities.observes_bash,
        loaded.memories.len(),
        core_count,
        preference_count,
        loaded.summaries.len(),
        loaded.workstreams.len(),
    ));
    Ok(())
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
) -> Result<(String, usize)> {
    let mut output = String::new();
    let limits = &policy.limits;
    let count = crate::preference::render_preferences_with_limits(
        &mut output,
        conn,
        project,
        cwd,
        limits.preference_project_limit,
        limits.preference_global_limit,
        limits.preference_char_limit,
    )?;
    Ok((output, count))
}

pub(in crate::context) fn enforce_total_char_limit(output: &mut String, char_limit: usize) {
    if char_limit == 0 || output.chars().count() <= char_limit {
        return;
    }

    let marker = "\n[remem context truncated to REMEM_CONTEXT_TOTAL_CHAR_LIMIT]\n";
    let marker_chars = marker.chars().count();
    if marker_chars >= char_limit {
        *output = output.chars().take(char_limit).collect();
        return;
    }

    let keep_chars = char_limit - marker_chars;
    let mut truncated: String = output.chars().take(keep_chars).collect();
    truncated.push_str(marker);
    *output = truncated;
}

fn build_context_header(project: &str, current_branch: Option<&str>) -> String {
    let branch_label = current_branch
        .map(|branch| format!(" @{}", branch))
        .unwrap_or_default();
    format!(
        "# [{}{}] context {}\n",
        project,
        branch_label,
        format_header_datetime()
    )
}
