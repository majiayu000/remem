//! Order-preserving helpers for the query search path, split out of
//! `text.rs` to keep it under the repository file-size ceiling.

use std::collections::HashMap;

use crate::memory::Memory;

use super::super::super::common::{weighted_rank_score, WeightedRankedChannel};
use super::super::{ChannelContribution, SearchWeights};
use super::{NamedChannel, QuerySearchPlan};

pub(super) fn log_search_timing(
    query_text: &str,
    project: Option<&str>,
    limit: i64,
    offset: i64,
    plan: &QuerySearchPlan,
) {
    crate::log::info(
        "search-perf",
        &format!(
            "query={} project={} limit={} offset={} fetch_limit={} {}",
            crate::db::truncate_str(query_text, 80),
            project.unwrap_or("-"),
            limit,
            offset,
            plan.fetch_limit,
            crate::perf::format_phase_timings(&plan.timings)
        ),
    );
}

pub(super) fn contributions_for(
    memory_id: i64,
    plan: &QuerySearchPlan,
) -> Vec<ChannelContribution> {
    plan.channels
        .iter()
        .filter_map(|channel| {
            channel
                .hits
                .iter()
                .position(|hit| hit.id == memory_id)
                .map(|index| ChannelContribution {
                    channel: channel.name.to_string(),
                    rank: index + 1,
                    score: weighted_rank_score(
                        channel.weight,
                        plan.weights.rrf_k,
                        index,
                        channel.hits[index].normalized_score,
                    ),
                })
        })
        .collect()
}

pub(super) fn weighted_channel_inputs(channels: &[NamedChannel]) -> Vec<WeightedRankedChannel<'_>> {
    channels
        .iter()
        .filter(|channel| channel.has_hits())
        .map(|channel| WeightedRankedChannel {
            weight: channel.weight,
            hits: &channel.hits,
        })
        .collect()
}

pub(super) fn retrieved_candidate_ids(channels: &[NamedChannel]) -> Vec<i64> {
    let mut ids = channels
        .iter()
        .filter(|channel| channel.has_hits())
        .flat_map(|channel| channel.hits.iter().map(|hit| hit.id))
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids
}

pub(super) fn apply_confidence_gate(
    fused: &[(i64, f64)],
    plan: &QuerySearchPlan,
    memories: &[Memory],
) -> Vec<(i64, f64)> {
    let min_confidence = plan.weights.min_evidence_confidence.clamp(0.0, 1.0);
    if min_confidence <= 0.0 || plan.claim_terms.is_empty() {
        return fused.to_vec();
    }
    let memory_by_id: HashMap<i64, &Memory> =
        memories.iter().map(|memory| (memory.id, memory)).collect();
    fused
        .iter()
        .copied()
        .filter(|(memory_id, _score)| {
            memory_by_id
                .get(memory_id)
                .is_some_and(|memory| candidate_confidence(memory, plan) >= min_confidence)
        })
        .collect()
}

pub(super) fn candidate_confidence(memory: &Memory, plan: &QuerySearchPlan) -> f64 {
    if plan.claim_terms.is_empty() || has_trusted_non_text_evidence(memory.id, plan) {
        return 1.0;
    }
    super::super::claim::claim_term_coverage(memory, &plan.claim_terms)
}

fn has_trusted_non_text_evidence(memory_id: i64, plan: &QuerySearchPlan) -> bool {
    let contributing: Vec<&str> = plan
        .channels
        .iter()
        .filter(|channel| channel.hits.iter().any(|hit| hit.id == memory_id))
        .map(|channel| channel.name)
        .filter(|name| *name != "usage")
        .collect();
    contributing.contains(&"fact")
        || contributing.contains(&"graph_traversal")
        || (!contributing.is_empty() && contributing.iter().all(|channel| *channel == "vector"))
}

pub(super) fn vector_similarity_score(distance: f32, weights: SearchWeights) -> f64 {
    let threshold = f64::from(weights.max_vector_distance);
    ((threshold - f64::from(distance)) / threshold).clamp(0.0, 1.0)
}

pub(super) fn visibility_label(memory: &Memory, requested_project: Option<&str>) -> &'static str {
    if memory.scope == "global" {
        "global-overlay"
    } else if requested_project
        .map(|project| crate::project_id::project_matches(Some(&memory.project), project))
        .unwrap_or(false)
    {
        "project-local"
    } else {
        "unscoped"
    }
}
