use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use rusqlite::OptionalExtension;

use super::{
    open_context_connection_or_error, render_context_load_errors,
    render_context_output_with_policy, section_render_limits,
};
use crate::context::policy::{ContextLimits, ContextPolicy, SectionKind};
use crate::context::query::load_context_data_with_policy;
use crate::context::render_inputs::render_preferences_to_buffer;
use crate::context::sections::{
    render_core_memory_with_limits_and_staleness, render_lessons_with_limit_and_staleness,
    render_memory_index_with_limits_excluding_and_staleness, render_recent_sessions_with_limit,
    render_workstreams_with_limits,
};
use crate::context::types::ContextRequest;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionStartBenchmarkEmission {
    pub rendered_output: String,
    pub injection_run_id: Option<String>,
}

pub(crate) fn session_start_benchmark_emission(
    cwd: &str,
    project: &str,
    host: &str,
) -> Result<SessionStartBenchmarkEmission> {
    static BENCHMARK_SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let sequence = BENCHMARK_SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let session_id = format!(
        "coding-bench-session-start-{}-{sequence}",
        std::process::id()
    );
    let invocation = crate::context::invocation::ContextInvocation {
        cwd: cwd.to_string(),
        project: project.to_string(),
        session_id: Some(session_id.clone()),
        transcript_path: None,
        source: Some("SessionStart".to_string()),
        host: crate::context::host::resolve_host_kind(Some(host)),
        use_colors: false,
        debug: false,
        force: true,
        gate_mode: Some("off".to_string()),
    };
    let rendered_output = super::generate_context_output_for_invocation(invocation, false)?;
    let conn = crate::db::open_db()?;
    let injection_run_id = conn
        .query_row(
            "SELECT a.injection_run_id
             FROM context_bundle_audits a
             JOIN context_injection_items i
               ON i.injection_run_id = a.injection_run_id
             WHERE i.project = ?1 AND i.session_id = ?2
             ORDER BY a.id DESC
             LIMIT 1",
            [project, session_id.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    Ok(SessionStartBenchmarkEmission {
        rendered_output,
        injection_run_id,
    })
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
        host: crate::context::host::resolve_host_kind(Some(host)),
        use_colors: false,
    };
    let policy = ContextPolicy::from_limits(ContextLimits::default());
    let rendered = match open_context_connection_or_error(&request, &policy) {
        Ok(conn) => render_context_output_with_policy(&conn, &request, None, false, policy, None)?,
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
    loaded: &crate::context::types::LoadedContext,
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
            loaded.render_reference_epoch,
            &loaded.staleness_labels,
        );
    }
    if !loaded.memories.is_empty() {
        let render_limits = section_render_limits(policy);
        let core_summary = render_core_memory_with_limits_and_staleness(
            &mut output,
            &loaded.memories,
            &render_limits,
            loaded.render_reference_epoch,
            &loaded.staleness_labels,
        );
        let core_ids: HashSet<i64> = core_summary.ids.into_iter().collect();
        render_memory_index_with_limits_excluding_and_staleness(
            &mut output,
            &loaded.memories,
            &render_limits,
            &core_ids,
            loaded.render_reference_epoch,
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
