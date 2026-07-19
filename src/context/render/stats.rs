use super::super::ownership::OwnerCounts;

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
