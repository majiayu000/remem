use super::super::ownership::OwnerCounts;
use super::super::types::{ContextRequest, LoadedContext};

pub(in crate::context) fn empty_stats(request: &ContextRequest) -> ContextRenderStats {
    ContextRenderStats {
        host: request.host.as_env_value().to_string(),
        branch: request.current_branch.clone(),
        hook_source: request.hook_source.clone(),
        ..ContextRenderStats::default()
    }
}

pub(in crate::context) fn empty_stats_with_load(
    request: &ContextRequest,
    loaded: &LoadedContext,
    load_timing: crate::perf::PhaseTiming,
    preference_timing: crate::perf::PhaseTiming,
) -> ContextRenderStats {
    let mut stats = empty_stats(request);
    absorb_evidence(
        &mut stats,
        load_timing,
        &loaded.load_phase_timings,
        preference_timing,
        loaded.rerank.as_ref(),
    );
    stats
}

/// GH-851 B-011/B-013: fold load/preference timings plus the shared rerank
/// stage evidence into the SessionStart render stats. The rerank phases use
/// the same names as standard search; error states are logged at error level
/// inside the shared stage itself.
pub(in crate::context) fn absorb_evidence(
    stats: &mut ContextRenderStats,
    load_timing: crate::perf::PhaseTiming,
    load_phase_timings: &[crate::perf::PhaseTiming],
    preference_timing: crate::perf::PhaseTiming,
    rerank: Option<&crate::retrieval::rerank::RerankExplain>,
) {
    stats.timings.push(load_timing);
    stats.timings.extend(load_phase_timings.iter().cloned());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absorb_evidence_preserves_load_subphase() {
        let mut stats = ContextRenderStats::default();
        absorb_evidence(
            &mut stats,
            crate::perf::PhaseTiming {
                phase: "load_context_data".to_string(),
                elapsed_ms: 11,
            },
            &[crate::perf::PhaseTiming {
                phase: "load_staleness_labels".to_string(),
                elapsed_ms: 7,
            }],
            crate::perf::PhaseTiming {
                phase: "load_preferences".to_string(),
                elapsed_ms: 3,
            },
            None,
        );

        assert_eq!(
            stats
                .timings
                .iter()
                .map(|timing| (timing.phase.as_str(), timing.elapsed_ms))
                .collect::<Vec<_>>(),
            vec![
                ("load_context_data", 11),
                ("load_staleness_labels", 7),
                ("load_preferences", 3),
            ]
        );
    }

    #[test]
    fn empty_stats_with_load_preserves_load_subphase() {
        let request = ContextRequest {
            cwd: "/repo".to_string(),
            project: "/repo".to_string(),
            session_id: None,
            hook_source: Some("startup".to_string()),
            current_branch: Some("main".to_string()),
            host: crate::context::host::HostKind::CodexCli,
            use_colors: false,
        };
        let loaded = LoadedContext {
            render_reference_epoch: 0,
            memories: Vec::new(),
            staleness_labels: Default::default(),
            lessons: Vec::new(),
            summaries: Vec::new(),
            workstreams: Vec::new(),
            relevance_query: None,
            memory_abstained: false,
            errors: Vec::new(),
            owner_traces: Vec::new(),
            owner_counts: Default::default(),
            diagnostics: Default::default(),
            load_phase_timings: vec![crate::perf::PhaseTiming {
                phase: "load_staleness_labels".to_string(),
                elapsed_ms: 5,
            }],
            rerank: None,
        };

        let stats = empty_stats_with_load(
            &request,
            &loaded,
            crate::perf::PhaseTiming {
                phase: "load_context_data".to_string(),
                elapsed_ms: 8,
            },
            crate::perf::PhaseTiming {
                phase: "load_preferences".to_string(),
                elapsed_ms: 2,
            },
        );

        assert!(stats
            .timings
            .iter()
            .any(|timing| timing.phase == "load_staleness_labels" && timing.elapsed_ms == 5));
    }
}
