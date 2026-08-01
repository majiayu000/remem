use std::collections::{HashMap, HashSet};
use std::time::Instant;

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};
use crate::perf::{push_elapsed, time_result, time_value, PhaseTiming};

use super::super::common::{
    calibrated_vector_hits, paginate_memories, sanitize_fts_query, weighted_ranked_fuse,
    WeightedRankedHit,
};
use super::{suppression_filter, SearchExplain, SearchWeights};

mod explain_build;
mod format;
mod graph;
mod support;
use explain_build::build_explain;
use format::fts_normalized_hits;
use support::{
    apply_confidence_gate, log_search_timing, resolve_explicit_entity_scope,
    resolve_graph_claim_support, retrieved_candidate_ids, weighted_channel_inputs,
};

pub(super) struct QuerySearchWithExplain {
    pub memories: Vec<Memory>,
    pub explain: SearchExplain,
}

struct QuerySearchPlan {
    expanded_terms: Vec<String>,
    core_terms: Vec<String>,
    claim_terms: Vec<String>,
    explicit_entity_terms: Vec<String>,
    explicit_entity_memory_ids: Vec<i64>,
    explicit_entity_neighbor_ids: Vec<i64>,
    fact_supported_memory_ids: Vec<i64>,
    graph_claim_supported_memory_ids: Vec<i64>,
    memory_type: Option<String>,
    branch: Option<String>,
    include_stale: bool,
    fts_query: Option<String>,
    temporal_range: Option<(i64, i64)>,
    temporal_field: Option<String>,
    include_suppressed: bool,
    fetch_limit: i64,
    weights: SearchWeights,
    channels: Vec<NamedChannel>,
    timings: Vec<PhaseTiming>,
}

struct NamedChannel {
    name: &'static str,
    weight: f64,
    disabled_reason: Option<String>,
    candidates_scanned: Option<usize>,
    embedding: Option<crate::retrieval::embedding::EmbeddingExecutionMetadata>,
    hits: Vec<WeightedRankedHit>,
}

impl NamedChannel {
    fn enabled(name: &'static str, weight: f64, ids: Vec<i64>) -> Self {
        let hits = ids.into_iter().map(WeightedRankedHit::rank_only).collect();
        Self::enabled_with_hits(name, weight, hits)
    }

    fn enabled_with_hits(name: &'static str, weight: f64, hits: Vec<WeightedRankedHit>) -> Self {
        Self {
            name,
            weight,
            disabled_reason: None,
            candidates_scanned: None,
            embedding: None,
            hits,
        }
    }

    fn disabled(name: &'static str, weight: f64, reason: impl Into<String>) -> Self {
        Self {
            name,
            weight,
            disabled_reason: Some(reason.into()),
            candidates_scanned: None,
            embedding: None,
            hits: vec![],
        }
    }

    fn with_candidates_scanned(mut self, candidates_scanned: usize) -> Self {
        self.candidates_scanned = Some(candidates_scanned);
        self
    }

    fn with_embedding(
        mut self,
        embedding: crate::retrieval::embedding::EmbeddingExecutionMetadata,
    ) -> Self {
        self.embedding = Some(embedding);
        self
    }

    fn is_enabled(&self) -> bool {
        self.disabled_reason.is_none()
    }

    fn has_hits(&self) -> bool {
        self.is_enabled() && !self.hits.is_empty()
    }
}

fn load_ordered_memories(
    conn: &Connection,
    ids: &[i64],
    project: Option<&str>,
    include_suppressed: bool,
) -> Result<Vec<Memory>> {
    let loaded =
        memory::get_memories_by_ids_with_suppressed_policy(conn, ids, project, include_suppressed)?;
    let id_to_memory: HashMap<i64, Memory> = loaded
        .into_iter()
        .map(|memory| (memory.id, memory))
        .collect();
    Ok(ids
        .iter()
        .filter_map(|id| id_to_memory.get(id).cloned())
        .collect())
}

fn gate_and_annotate_memories(
    conn: &Connection,
    query_text: &str,
    project: Option<&str>,
    fused: &[(i64, f64)],
    plan: &mut QuerySearchPlan,
    ordered: Vec<Memory>,
) -> Result<(Vec<Memory>, Vec<(i64, f64)>)> {
    let (ordered, fused) =
        suppression_filter::ordered(conn, ordered, fused, plan.include_suppressed)?;
    resolve_explicit_entity_scope(conn, query_text, project, &fused, plan, &ordered)?;
    resolve_graph_claim_support(conn, project, plan)?;
    let gated_fused = apply_confidence_gate(&fused, plan, &ordered);
    let gated_ids: HashSet<i64> = gated_fused.iter().map(|(id, _)| *id).collect();
    let mut ordered = ordered
        .into_iter()
        .filter(|memory| gated_ids.contains(&memory.id))
        .collect::<Vec<_>>();
    crate::retrieval::temporal::annotate_memories_with_fact_labels(
        conn,
        &mut ordered,
        Some(query_text),
        project,
    )?;
    Ok((ordered, gated_fused))
}

pub(super) fn search_with_query(
    conn: &Connection,
    query_text: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
    include_suppressed: bool,
) -> Result<Vec<Memory>> {
    search_with_query_weights(
        conn,
        query_text,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
        include_suppressed,
        SearchWeights::default(),
    )
}

pub(super) fn search_with_query_weights(
    conn: &Connection,
    query_text: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
    include_suppressed: bool,
    weights: SearchWeights,
) -> Result<Vec<Memory>> {
    let mut plan = build_query_search_plan(
        conn,
        query_text,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
        include_suppressed,
        weights,
    )?;
    if plan.channels.is_empty() {
        log_search_timing(query_text, project, limit, offset, &plan);
        return Ok(vec![]);
    }

    let channel_inputs = time_value(&mut plan.timings, "fusion_inputs", || {
        weighted_channel_inputs(&plan.channels)
    });
    let fused = time_result(&mut plan.timings, "rrf_fusion", || {
        weighted_ranked_fuse(&channel_inputs, plan.weights.rrf_k)
    })?;
    let fused_ids: Vec<i64> = fused.iter().map(|(id, _)| *id).collect();
    let ordered = time_result(&mut plan.timings, "load_memories", || {
        load_ordered_memories(conn, &fused_ids, project, plan.include_suppressed)
    })?;
    let (ordered, fused) = time_result(&mut plan.timings, "source_anchor_demote", || {
        super::source_anchor::apply_score_demotions(conn, &fused, ordered)
    })?;
    let annotate_start = Instant::now();
    let (ordered, _) =
        gate_and_annotate_memories(conn, query_text, project, &fused, &mut plan, ordered)?;
    push_elapsed(
        &mut plan.timings,
        "confidence_and_fact_labels",
        annotate_start,
    );
    let (ordered, _rerank_outcome) =
        apply_rerank_stage(conn, query_text, ordered, &mut plan.timings)?;
    let paged = time_value(&mut plan.timings, "paginate", || {
        paginate_memories(ordered, limit, offset)
    });
    log_search_timing(query_text, project, limit, offset, &plan);
    Ok(paged)
}

pub(super) fn search_with_query_explain(
    conn: &Connection,
    query_text: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
    include_suppressed: bool,
) -> Result<QuerySearchWithExplain> {
    let mut plan = build_query_search_plan(
        conn,
        query_text,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
        include_suppressed,
        SearchWeights::default(),
    )?;
    if plan.channels.is_empty() {
        log_search_timing(query_text, project, limit, offset, &plan);
        return Ok(QuerySearchWithExplain {
            memories: vec![],
            explain: SearchExplain {
                query: query_text.to_string(),
                project: project.map(str::to_string),
                memory_type: memory_type.map(str::to_string),
                branch: branch.map(str::to_string),
                include_stale,
                limit,
                offset,
                fetch_limit: plan.fetch_limit,
                expanded_terms: plan.expanded_terms,
                core_terms: plan.core_terms,
                claim_terms: plan.claim_terms,
                fts_query: plan.fts_query,
                temporal_range: plan.temporal_range,
                temporal_field: plan.temporal_field,
                rrf_k: plan.weights.rrf_k,
                min_evidence_confidence: plan.weights.min_evidence_confidence,
                filtered_result_count: 0,
                timings: plan.timings,
                rerank: None,
                channels: vec![],
                results: vec![],
                has_more: false,
                raw_fallback_count: 0,
            },
        });
    }

    let channel_inputs = time_value(&mut plan.timings, "fusion_inputs", || {
        weighted_channel_inputs(&plan.channels)
    });
    let fused = time_result(&mut plan.timings, "rrf_fusion", || {
        weighted_ranked_fuse(&channel_inputs, plan.weights.rrf_k)
    })?;
    let fusion_scores = fused.clone();
    let fused_ids: Vec<i64> = fused.iter().map(|(id, _)| *id).collect();
    let ordered = time_result(&mut plan.timings, "load_memories", || {
        load_ordered_memories(conn, &fused_ids, project, plan.include_suppressed)
    })?;
    let (ordered, fused) = time_result(&mut plan.timings, "source_anchor_demote", || {
        super::source_anchor::apply_score_demotions(conn, &fused, ordered)
    })?;
    let annotate_start = Instant::now();
    let (ordered, gated_fused) =
        gate_and_annotate_memories(conn, query_text, project, &fused, &mut plan, ordered)?;
    push_elapsed(
        &mut plan.timings,
        "confidence_and_fact_labels",
        annotate_start,
    );
    let (ordered, rerank_outcome) =
        apply_rerank_stage(conn, query_text, ordered, &mut plan.timings)?;
    let paged = time_value(&mut plan.timings, "paginate", || {
        paginate_memories(ordered, limit, offset)
    });
    let explain_start = Instant::now();
    let mut explain = build_explain(
        conn,
        query_text,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
        &plan,
        &fusion_scores,
        &gated_fused,
        fusion_scores.len().saturating_sub(gated_fused.len()),
        &paged,
    )?;
    explain.rerank = Some(rerank_explain(&rerank_outcome));
    push_elapsed(&mut plan.timings, "build_explain", explain_start);
    log_search_timing(query_text, project, limit, offset, &plan);
    Ok(QuerySearchWithExplain {
        memories: paged,
        explain,
    })
}

fn build_query_search_plan(
    conn: &Connection,
    query_text: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
    include_suppressed: bool,
    weights: SearchWeights,
) -> Result<QuerySearchPlan> {
    weights.validate()?;
    let total_start = Instant::now();
    let mut timings = Vec::new();
    let page_target = (limit.max(1) + offset.max(0) + 1).max(2);
    let fetch = page_target * 3;
    let expanded = time_value(&mut timings, "query_expand", || {
        crate::retrieval::query_expand::expand_query(query_text)
    });
    let expanded_refs: Vec<&str> = expanded.iter().map(|token| token.as_str()).collect();
    let long_tokens: Vec<&str> = expanded_refs
        .iter()
        .filter(|token| token.chars().count() >= 3)
        .copied()
        .collect();

    let core_tokens = crate::retrieval::query_expand::core_tokens(query_text);
    let entity_candidates = super::claim::entity_scope_candidates(query_text, project);
    let (explicit_entity_terms, explicit_entity_ids) =
        time_result(&mut timings, "explicit_entity_anchor", || {
            super::claim::select_entity_anchors(
                conn,
                &entity_candidates,
                project,
                memory_type,
                branch,
                fetch,
                include_stale,
                include_suppressed,
            )
        })?;
    let claim_terms = super::claim::claim_terms(&core_tokens, project, &explicit_entity_terms);
    let core_refs: Vec<&str> = core_tokens.iter().map(|token| token.as_str()).collect();
    let mut channels: Vec<NamedChannel> = Vec::new();
    let mut fts_query = None;
    let mut temporal_range = None;
    let mut temporal_field = None;

    if !long_tokens.is_empty() {
        let safe_query = sanitize_fts_query(&long_tokens.join(" "));
        fts_query = Some(safe_query.clone());
        let fts = time_result(&mut timings, "fts", || {
            memory::search_memories_fts_hits_filtered(
                conn,
                &safe_query,
                project,
                memory_type,
                fetch,
                0,
                include_stale,
                branch,
            )
        })?;
        let fts = suppression_filter::fts_hits(conn, fts, include_suppressed)?;
        if !fts.is_empty() {
            channels.push(NamedChannel::enabled_with_hits(
                "fts",
                weights.fts,
                fts_normalized_hits(&fts),
            ));
        }
    }

    let entity_ids = time_result(&mut timings, "entity", || {
        crate::retrieval::entity::search_by_entity_filtered(
            conn,
            query_text,
            project,
            memory_type,
            branch,
            fetch,
            include_stale,
        )
    })?;
    let entity_ids = suppression_filter::ids(conn, entity_ids, include_suppressed)?;
    if !entity_ids.is_empty() {
        channels.push(NamedChannel::enabled("entity", weights.entity, entity_ids));
    }

    if weights.fact > 0.0 {
        let fact_ids = time_result(&mut timings, "fact", || {
            crate::retrieval::temporal::search_fact_memory_ids(
                conn,
                &core_refs,
                project,
                memory_type,
                &[],
                None,
                branch,
                fetch,
                include_stale,
                crate::retrieval::temporal::FactTimeMode::from_query(query_text),
            )
        })?;
        let fact_ids = suppression_filter::ids(conn, fact_ids, include_suppressed)?;
        if !fact_ids.is_empty() {
            channels.push(NamedChannel::enabled("fact", weights.fact, fact_ids));
        }
    }

    if let Some(temporal_constraint) = crate::retrieval::temporal::extract_temporal(query_text) {
        temporal_range = Some((
            temporal_constraint.start_epoch,
            temporal_constraint.end_epoch,
        ));
        temporal_field = Some(temporal_constraint.field.as_str().to_string());
        let temporal_ids = time_result(&mut timings, "temporal", || {
            crate::retrieval::temporal::search_by_time_filtered(
                conn,
                &temporal_constraint,
                project,
                memory_type,
                branch,
                fetch,
                include_stale,
            )
        })?;
        let temporal_ids = suppression_filter::ids(conn, temporal_ids, include_suppressed)?;
        if !temporal_ids.is_empty() {
            channels.push(NamedChannel::enabled(
                "temporal",
                weights.temporal,
                temporal_ids,
            ));
        }
    }

    let query_embedding = time_result(&mut timings, "query_embedding", || {
        crate::retrieval::embedding::embed_query_with_execution_if_enabled(query_text)
    })?;
    if let Some(query_embedding) = query_embedding {
        let crate::retrieval::embedding::QueryEmbeddingExecution {
            embedding,
            metadata,
        } = query_embedding;
        let vector_start = Instant::now();
        let mut vector_outcome = crate::retrieval::vector::vector_search_embedding_filtered(
            conn,
            &embedding,
            crate::retrieval::vector::VectorSearchFilters {
                project,
                memory_type,
                branch,
                include_stale,
            },
            fetch as usize,
        )?;
        push_elapsed(&mut timings, "vector", vector_start);
        timings.append(&mut vector_outcome.timings);
        let channel = if let Some(reason) = vector_outcome.disabled_reason {
            NamedChannel::disabled("vector", weights.vector, reason)
                .with_candidates_scanned(vector_outcome.candidates_scanned)
        } else {
            let candidates_scanned = vector_outcome.candidates_scanned;
            let hits = calibrated_vector_hits(
                vector_outcome
                    .hits
                    .into_iter()
                    .map(|hit| (hit.memory_id, hit.distance)),
                weights.max_vector_distance,
            )?;
            let hits = suppression_filter::weighted_hits(conn, hits, include_suppressed)?;
            NamedChannel::enabled_with_hits("vector", weights.vector, hits)
                .with_candidates_scanned(candidates_scanned)
        };
        channels.push(channel.with_embedding(metadata));
    } else {
        channels.push(NamedChannel::disabled(
            "vector",
            weights.vector,
            "embedding provider is off",
        ));
    }

    graph::append_graph_channel(
        conn,
        &mut channels,
        &mut timings,
        project,
        memory_type,
        branch,
        include_stale,
        include_suppressed,
        fetch,
        weights,
    )?;

    if core_refs.is_empty() {
        channels.push(NamedChannel::disabled(
            "like_fallback",
            weights.like_fallback,
            "no core terms for LIKE fallback",
        ));
    } else if channels.iter().any(NamedChannel::has_hits) {
        channels.push(NamedChannel::disabled(
            "like_fallback",
            weights.like_fallback,
            "stronger retrieval channels returned hits",
        ));
    } else {
        let like = time_result(&mut timings, "like_fallback", || {
            memory::search_memories_like_filtered(
                conn,
                &core_refs,
                project,
                memory_type,
                fetch,
                0,
                include_stale,
                branch,
            )
        })?;
        let like = suppression_filter::memories(conn, like, include_suppressed)?;
        if like.is_empty() {
            channels.push(NamedChannel::disabled(
                "like_fallback",
                weights.like_fallback,
                "LIKE fallback returned no hits",
            ));
        } else {
            channels.push(NamedChannel::enabled(
                "like_fallback",
                weights.like_fallback,
                like.iter().map(|memory| memory.id).collect(),
            ));
        }
    }

    if weights.usage > 0.0 {
        let usage_candidates = retrieved_candidate_ids(&channels);
        let usage_hits = time_result(&mut timings, "usage", || {
            super::usage_rank::usage_hits_for_retrieved_candidates(conn, &usage_candidates, weights)
        })?;
        if usage_hits.is_empty() {
            channels.push(NamedChannel::disabled(
                "usage",
                weights.usage,
                "no retrieved candidates with usage signals",
            ));
        } else {
            channels.push(NamedChannel::enabled_with_hits(
                "usage",
                weights.usage,
                usage_hits,
            ));
        }
    }

    push_elapsed(&mut timings, "plan_total", total_start);
    Ok(QuerySearchPlan {
        expanded_terms: expanded,
        core_terms: core_tokens,
        claim_terms,
        explicit_entity_terms,
        explicit_entity_memory_ids: explicit_entity_ids,
        explicit_entity_neighbor_ids: Vec::new(),
        fact_supported_memory_ids: Vec::new(),
        graph_claim_supported_memory_ids: Vec::new(),
        memory_type: memory_type.map(str::to_string),
        branch: branch.map(str::to_string),
        include_stale,
        fts_query,
        temporal_range,
        temporal_field,
        include_suppressed,
        fetch_limit: fetch,
        weights,
        channels,
        timings,
    })
}

/// Shared post-eligibility rerank hook (GH-851): runs after the confidence
/// gate and source-anchor demotion, before pagination. On `Applied` the
/// returned order is the fixed top-k result set; on any other status the
/// complete baseline order passes through unchanged.
fn apply_rerank_stage(
    conn: &Connection,
    query_text: &str,
    ordered: Vec<Memory>,
    timings: &mut Vec<PhaseTiming>,
) -> Result<(Vec<Memory>, crate::retrieval::rerank::types::RerankOutcome)> {
    let (ordered, outcome) = crate::retrieval::rerank::apply_to_search(conn, query_text, ordered)?;
    timings.extend(outcome.timings.iter().cloned());
    Ok((ordered, outcome))
}

fn rerank_explain(
    outcome: &crate::retrieval::rerank::types::RerankOutcome,
) -> crate::retrieval::rerank::RerankExplain {
    let requested =
        outcome.disabled_reason() != Some(crate::retrieval::rerank::RerankDisabledReason::Off);
    outcome.to_explain(requested)
}

#[cfg(test)]
mod tests;
