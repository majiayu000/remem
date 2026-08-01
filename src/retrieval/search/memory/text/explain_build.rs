use std::collections::HashMap;

use anyhow::{ensure, Context, Result};
use rusqlite::Connection;

use crate::memory::{self, Memory};

use super::support::{candidate_confidence, contributions_for, visibility_label};
use super::QuerySearchPlan;
use crate::retrieval::search::{
    ChannelHit, SearchExplain, SearchExplainChannel, SearchExplainResult,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn build_explain(
    conn: &Connection,
    query_text: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
    plan: &QuerySearchPlan,
    fusion_scores: &[(i64, f64)],
    final_scores: &[(i64, f64)],
    filtered_result_count: usize,
    paged: &[Memory],
) -> Result<SearchExplain> {
    let channels = plan
        .channels
        .iter()
        .map(|channel| SearchExplainChannel {
            name: channel.name.to_string(),
            enabled: channel.is_enabled(),
            disabled_reason: channel.disabled_reason.clone(),
            candidates_scanned: channel.candidates_scanned,
            embedding: channel.embedding.clone(),
            hits: channel
                .hits
                .iter()
                .enumerate()
                .map(|(index, hit)| ChannelHit {
                    memory_id: hit.id,
                    rank: index + 1,
                })
                .collect(),
        })
        .collect();
    let id_to_fusion_score: HashMap<i64, f64> = fusion_scores.iter().copied().collect();
    let id_to_final_score: HashMap<i64, f64> = final_scores.iter().copied().collect();
    let now_epoch = chrono::Utc::now().timestamp();
    let staleness_labels =
        memory::staleness::memory_staleness_labels_for_memories(conn, paged, now_epoch)?;
    let explained_results = paged
        .iter()
        .enumerate()
        .map(|(index, memory)| {
            explain_result(
                memory,
                index,
                project,
                plan,
                &id_to_fusion_score,
                &id_to_final_score,
                &staleness_labels,
                now_epoch,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let results = explained_results;
    Ok(SearchExplain {
        query: query_text.to_string(),
        project: project.map(str::to_string),
        memory_type: memory_type.map(str::to_string),
        branch: branch.map(str::to_string),
        include_stale,
        limit,
        offset,
        fetch_limit: plan.fetch_limit,
        expanded_terms: plan.expanded_terms.clone(),
        core_terms: plan.core_terms.clone(),
        claim_terms: plan.claim_terms.clone(),
        fts_query: plan.fts_query.clone(),
        temporal_range: plan.temporal_range,
        temporal_field: plan.temporal_field.clone(),
        rrf_k: plan.weights.rrf_k,
        min_evidence_confidence: plan.weights.min_evidence_confidence,
        filtered_result_count,
        timings: plan.timings.clone(),
        rerank: None,
        channels,
        results,
        has_more: false,
        raw_fallback_count: 0,
    })
}

#[allow(clippy::too_many_arguments)]
fn explain_result(
    memory: &Memory,
    index: usize,
    project: Option<&str>,
    plan: &QuerySearchPlan,
    id_to_fusion_score: &HashMap<i64, f64>,
    id_to_final_score: &HashMap<i64, f64>,
    staleness_labels: &HashMap<i64, crate::memory::MemoryStalenessLabel>,
    now_epoch: i64,
) -> Result<SearchExplainResult> {
    let final_score = id_to_final_score
        .get(&memory.id)
        .copied()
        .with_context(|| format!("missing final score for memory {}", memory.id))?;
    ensure!(
        final_score.is_finite() && final_score >= 0.0,
        "invalid final score {final_score} for memory {}",
        memory.id
    );
    let contributions = contributions_for(memory.id, plan)?;
    let contribution_sum = contributions.iter().try_fold(0.0, |total, contribution| {
        let next = total + contribution.score;
        ensure!(
            next.is_finite(),
            "fusion explanation score overflow for memory {}",
            memory.id
        );
        Ok(next)
    })?;
    let fusion_score = id_to_fusion_score
        .get(&memory.id)
        .copied()
        .with_context(|| format!("missing fusion score for memory {}", memory.id))?;
    ensure!(
        fusion_score.is_finite() && fusion_score > 0.0,
        "invalid fusion score {fusion_score} for memory {}",
        memory.id
    );
    let score_tolerance = f64::EPSILON * 16.0 * fusion_score.abs().max(1.0);
    ensure!(
        (fusion_score - contribution_sum).abs() <= score_tolerance,
        "fusion contribution mismatch for memory {}: fused={fusion_score} explained={contribution_sum}",
        memory.id
    );
    let post_fusion_score_factor = final_score / fusion_score;
    ensure!(
        post_fusion_score_factor.is_finite() && post_fusion_score_factor >= 0.0,
        "invalid post-fusion score factor for memory {}",
        memory.id
    );
    Ok(SearchExplainResult {
        memory_id: memory.id,
        final_rank: index + 1,
        final_score,
        evidence_confidence: candidate_confidence(memory, plan),
        project: memory.project.clone(),
        scope: memory.scope.clone(),
        visibility: visibility_label(memory, project).to_string(),
        staleness: staleness_labels
            .get(&memory.id)
            .cloned()
            .unwrap_or_else(|| memory::memory_staleness_label(memory, now_epoch)),
        contributions,
    })
}
