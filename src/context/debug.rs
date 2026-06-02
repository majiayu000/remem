use super::format::truncate_chars_with_ellipsis;
use super::policy::ContextPolicy;
use super::render::ContextRenderStats;
use super::types::{ContextRequest, LoadedContext};

pub(in crate::context) fn build_context_debug_trace(
    request: &ContextRequest,
    policy: &ContextPolicy,
    loaded: &LoadedContext,
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
        "- context diagnostics candidate_pool_total={} current_rows={} selected_ids=[{}]\n",
        loaded.diagnostics.candidate_pool_total,
        loaded.diagnostics.current_rows,
        format_id_list(&loaded.diagnostics.selected_ids)
    ));
    for group in &loaded.diagnostics.hidden_duplicate_groups {
        trace.push_str(&format!(
            "- hidden duplicate group key={} chosen=#{} hidden=[{}] reason=near_duplicate_legacy_rows\n",
            group.cluster_key,
            group.chosen_id,
            format_id_list(&group.hidden_ids)
        ));
    }
    trace.push_str(&format!(
        "- preference diagnostics selected_ids=[{}]\n",
        format_id_list(&loaded.diagnostics.preference_selected_ids)
    ));
    for group in &loaded.diagnostics.preference_hidden_duplicate_groups {
        trace.push_str(&format!(
            "- hidden preference duplicate group key={} chosen=#{} hidden=[{}] reason=same_owner_topic_key\n",
            group.cluster_key,
            group.chosen_id,
            format_id_list(&group.hidden_ids)
        ));
    }
    for group in &loaded.diagnostics.preference_state_key_groups {
        trace.push_str(&format!(
            "- preference state-key group owner={}:{} type={} key={} current={} active=[{}] reason={}\n",
            group.owner_scope,
            group.owner_key,
            group.memory_type,
            group.state_key,
            group
                .current_id
                .map(|id| format!("#{id}"))
                .unwrap_or_else(|| "-".to_string()),
            format_id_list(&group.active_ids),
            group.reason
        ));
    }
    for group in &loaded.diagnostics.state_key_groups {
        trace.push_str(&format!(
            "- state-key group owner={}:{} type={} key={} current={} active=[{}] reason={}\n",
            group.owner_scope,
            group.owner_key,
            group.memory_type,
            group.state_key,
            group
                .current_id
                .map(|id| format!("#{id}"))
                .unwrap_or_else(|| "-".to_string()),
            format_id_list(&group.active_ids),
            group.reason
        ));
    }
    for exclusion in &loaded.diagnostics.exclusions {
        trace.push_str(&format!(
            "- excluded memory id={} reason={} status={} title={}\n",
            exclusion.id,
            exclusion.reason,
            exclusion.status,
            truncate_chars_with_ellipsis(&exclusion.title, 120)
        ));
    }
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

fn format_id_list(ids: &[i64]) -> String {
    ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
}
