use super::super::ownership::OwnerCounts;
use super::super::types::ContextRequest;

pub(in crate::context) fn empty_stats(request: &ContextRequest) -> ContextRenderStats {
    ContextRenderStats {
        host: request.host.as_env_value().to_string(),
        branch: request.current_branch.clone(),
        hook_source: request.hook_source.clone(),
        ..ContextRenderStats::default()
    }
}

/// GH-851 B-011/B-013: fold load/preference timings plus the shared rerank
/// stage evidence into the SessionStart render stats. The rerank phases use
/// the same names as standard search; error states are logged at error level
/// inside the shared stage itself.
pub(in crate::context) fn absorb_evidence(
    stats: &mut ContextRenderStats,
    load_timing: crate::perf::PhaseTiming,
    preference_timing: crate::perf::PhaseTiming,
    rerank: Option<&crate::retrieval::rerank::RerankExplain>,
) {
    stats.timings.push(load_timing);
    stats.timings.push(preference_timing);
    let Some(rerank) = rerank else {
        return;
    };
    stats.timings.extend(rerank.timings.iter().cloned());
    crate::log::info(
        "context",
        &format!(
            "rerank requested={} applied={} reason={} input={} output={}",
            rerank.requested,
            rerank.applied,
            rerank.disabled_reason.as_deref().unwrap_or("-"),
            rerank.input_count,
            rerank.output_count
        ),
    );
}

#[derive(Debug, Clone, Default)]
pub(in crate::context) struct SectionRenderStats {
    pub count: usize,
    pub chars: usize,
}

#[derive(Debug, Clone)]
pub(in crate::context) struct RelevanceRenderStats {
    pub state: &'static str,
    pub k: usize,
    pub threshold: Option<f64>,
    pub candidates: usize,
    pub eligible: usize,
    pub final_injected: usize,
    pub below_threshold: usize,
    pub k_limited: usize,
    pub section_limited: usize,
    pub total_limited: usize,
}

impl Default for RelevanceRenderStats {
    fn default() -> Self {
        Self {
            state: "unavailable",
            k: 0,
            threshold: None,
            candidates: 0,
            eligible: 0,
            final_injected: 0,
            below_threshold: 0,
            k_limited: 0,
            section_limited: 0,
            total_limited: 0,
        }
    }
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
    pub relevance: RelevanceRenderStats,
    pub owner_counts: OwnerCounts,
    pub core_ids: Vec<i64>,
    pub output_chars: usize,
    pub truncated: bool,
    pub timings: Vec<crate::perf::PhaseTiming>,
}
