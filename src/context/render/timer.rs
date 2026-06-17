use super::ContextRenderStats;
use crate::context::host::resolve_profile;
use crate::context::injection_gate::{ContextGateDecision, ContextGatePrecheck};
use crate::context::types::ContextRequest;

pub(super) fn log_context_timer(
    timer: crate::log::Timer,
    request: &ContextRequest,
    decision: &ContextGateDecision,
    stats: &ContextRenderStats,
    precheck: ContextGatePrecheck,
) {
    let capabilities = resolve_profile(request.host).capabilities();
    timer.done(&format!(
        "project={} cwd={} session={} host={} colors={} gate={:?}:{} precheck={} caps=[mcp:{} session_start:{} prompt_submit:{} native_edits:{} bash:{}] context_memories={} core={} lessons={} index={} preferences={} sessions={} workstreams={} timings=[{}]",
        request.project,
        request.cwd,
        request.session_id.as_deref().unwrap_or("-"),
        request.host.as_env_value(),
        request.use_colors,
        decision.action,
        decision.reason,
        precheck.as_log_value(),
        capabilities.has_mcp_tools,
        capabilities.has_session_start_hook,
        capabilities.has_user_prompt_submit_hook,
        capabilities.observes_native_file_edits,
        capabilities.observes_bash,
        stats.memories_loaded,
        stats.core.count,
        stats.lessons.count,
        stats.index.count,
        stats.preferences.count,
        stats.sessions.count,
        stats.workstreams.count,
        crate::perf::format_phase_timings(&stats.timings),
    ));
}
