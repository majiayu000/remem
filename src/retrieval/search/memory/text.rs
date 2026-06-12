use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};

use super::super::common::{
    paginate_memories, rank_normalized_score, sanitize_fts_query, weighted_rank_score,
    weighted_ranked_fuse, WeightedRankedChannel, WeightedRankedHit,
};
use super::{
    ChannelContribution, ChannelHit, SearchExplain, SearchExplainChannel, SearchExplainResult,
};

const RRF_K: f64 = 60.0;
const MAX_VECTOR_DISTANCE: f32 = 0.51;
const FTS_WEIGHT: f64 = 2.5;
const VECTOR_WEIGHT: f64 = 3.0;
const ENTITY_WEIGHT: f64 = 1.25;
const TEMPORAL_WEIGHT: f64 = 1.0;
const LIKE_FALLBACK_WEIGHT: f64 = 0.25;
const MIN_EVIDENCE_CONFIDENCE: f64 = 0.62;

#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SearchWeights {
    pub fts: f64,
    pub vector: f64,
    pub entity: f64,
    pub temporal: f64,
    pub like_fallback: f64,
    pub max_vector_distance: f32,
    pub rrf_k: f64,
    pub min_evidence_confidence: f64,
}

impl Default for SearchWeights {
    fn default() -> Self {
        Self {
            fts: FTS_WEIGHT,
            vector: VECTOR_WEIGHT,
            entity: ENTITY_WEIGHT,
            temporal: TEMPORAL_WEIGHT,
            like_fallback: LIKE_FALLBACK_WEIGHT,
            max_vector_distance: MAX_VECTOR_DISTANCE,
            rrf_k: RRF_K,
            min_evidence_confidence: MIN_EVIDENCE_CONFIDENCE,
        }
    }
}

pub(super) struct QuerySearchWithExplain {
    pub memories: Vec<Memory>,
    pub explain: SearchExplain,
}

struct QuerySearchPlan {
    expanded_terms: Vec<String>,
    core_terms: Vec<String>,
    claim_terms: Vec<String>,
    fts_query: Option<String>,
    temporal_range: Option<(i64, i64)>,
    temporal_field: Option<String>,
    fetch_limit: i64,
    weights: SearchWeights,
    channels: Vec<NamedChannel>,
}

struct NamedChannel {
    name: &'static str,
    weight: f64,
    disabled_reason: Option<String>,
    hits: Vec<WeightedRankedHit>,
}

impl NamedChannel {
    fn enabled(name: &'static str, weight: f64, ids: Vec<i64>) -> Self {
        let hits = ids
            .into_iter()
            .enumerate()
            .map(|(rank, id)| WeightedRankedHit {
                id,
                normalized_score: rank_normalized_score(rank),
            })
            .collect();
        Self::enabled_with_hits(name, weight, hits)
    }

    fn enabled_with_hits(name: &'static str, weight: f64, hits: Vec<WeightedRankedHit>) -> Self {
        Self {
            name,
            weight,
            disabled_reason: None,
            hits,
        }
    }

    fn disabled(name: &'static str, weight: f64, reason: impl Into<String>) -> Self {
        Self {
            name,
            weight,
            disabled_reason: Some(reason.into()),
            hits: vec![],
        }
    }

    fn is_enabled(&self) -> bool {
        self.disabled_reason.is_none()
    }

    fn has_hits(&self) -> bool {
        self.is_enabled() && !self.hits.is_empty()
    }
}

fn load_ordered_memories(conn: &Connection, ids: &[i64]) -> Result<Vec<Memory>> {
    let loaded = memory::get_memories_by_ids(conn, ids, None)?;
    let id_to_memory: HashMap<i64, Memory> = loaded
        .into_iter()
        .map(|memory| (memory.id, memory))
        .collect();
    Ok(ids
        .iter()
        .filter_map(|id| id_to_memory.get(id).cloned())
        .collect())
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
        SearchWeights::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn search_with_query_weights(
    conn: &Connection,
    query_text: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
    weights: SearchWeights,
) -> Result<Vec<Memory>> {
    let plan = build_query_search_plan(
        conn,
        query_text,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
        weights,
    )?;
    if plan.channels.is_empty() {
        return Ok(vec![]);
    }

    let channel_inputs = weighted_channel_inputs(&plan.channels);
    let fused = weighted_ranked_fuse(&channel_inputs, plan.weights.rrf_k);
    let fused_ids: Vec<i64> = fused.iter().map(|(id, _)| *id).collect();
    let ordered = load_ordered_memories(conn, &fused_ids)?;
    let gated_fused = apply_confidence_gate(&fused, &plan, &ordered);
    let gated_ids: HashSet<i64> = gated_fused.iter().map(|(id, _)| *id).collect();
    let ordered: Vec<Memory> = ordered
        .into_iter()
        .filter(|memory| gated_ids.contains(&memory.id))
        .collect();
    Ok(paginate_memories(ordered, limit, offset))
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
) -> Result<QuerySearchWithExplain> {
    let plan = build_query_search_plan(
        conn,
        query_text,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
        SearchWeights::default(),
    )?;
    if plan.channels.is_empty() {
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
                channels: vec![],
                results: vec![],
                has_more: false,
                raw_fallback_count: 0,
            },
        });
    }

    let channel_inputs = weighted_channel_inputs(&plan.channels);
    let fused = weighted_ranked_fuse(&channel_inputs, plan.weights.rrf_k);
    let fused_ids: Vec<i64> = fused.iter().map(|(id, _)| *id).collect();
    let ordered = load_ordered_memories(conn, &fused_ids)?;
    let gated_fused = apply_confidence_gate(&fused, &plan, &ordered);
    let gated_ids: HashSet<i64> = gated_fused.iter().map(|(id, _)| *id).collect();
    let ordered: Vec<Memory> = ordered
        .into_iter()
        .filter(|memory| gated_ids.contains(&memory.id))
        .collect();
    let paged = paginate_memories(ordered, limit, offset);
    let explain = build_explain(
        query_text,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
        &plan,
        &gated_fused,
        fused.len().saturating_sub(gated_fused.len()),
        &paged,
    );
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
    weights: SearchWeights,
) -> Result<QuerySearchPlan> {
    let page_target = (limit.max(1) + offset.max(0) + 1).max(2);
    let fetch = page_target * 3;
    let expanded = crate::retrieval::query_expand::expand_query(query_text);
    let expanded_refs: Vec<&str> = expanded.iter().map(|token| token.as_str()).collect();
    let long_tokens: Vec<&str> = expanded_refs
        .iter()
        .filter(|token| token.chars().count() >= 3)
        .copied()
        .collect();

    let core_tokens = crate::retrieval::query_expand::core_tokens(query_text);
    let claim_terms = claim_terms(query_text, &core_tokens, project);
    let core_refs: Vec<&str> = core_tokens.iter().map(|token| token.as_str()).collect();
    let mut channels: Vec<NamedChannel> = Vec::new();
    let mut fts_query = None;
    let mut temporal_range = None;
    let mut temporal_field = None;

    if !long_tokens.is_empty() {
        let safe_query = sanitize_fts_query(&long_tokens.join(" "));
        fts_query = Some(safe_query.clone());
        let fts = memory::search_memories_fts_hits_filtered(
            conn,
            &safe_query,
            project,
            memory_type,
            fetch,
            0,
            include_stale,
            branch,
        )?;
        if !fts.is_empty() {
            channels.push(NamedChannel::enabled_with_hits(
                "fts",
                weights.fts,
                fts_normalized_hits(&fts),
            ));
        }
    }

    let entity_ids = crate::retrieval::entity::search_by_entity_filtered(
        conn,
        query_text,
        project,
        memory_type,
        branch,
        fetch,
        include_stale,
    )?;
    if !entity_ids.is_empty() {
        channels.push(NamedChannel::enabled("entity", weights.entity, entity_ids));
    }

    if let Some(temporal_constraint) = crate::retrieval::temporal::extract_temporal(query_text) {
        temporal_range = Some((
            temporal_constraint.start_epoch,
            temporal_constraint.end_epoch,
        ));
        temporal_field = Some(temporal_constraint.field.as_str().to_string());
        let temporal_ids = crate::retrieval::temporal::search_by_time_filtered(
            conn,
            &temporal_constraint,
            project,
            memory_type,
            branch,
            fetch,
            include_stale,
        )?;
        if !temporal_ids.is_empty() {
            channels.push(NamedChannel::enabled(
                "temporal",
                weights.temporal,
                temporal_ids,
            ));
        }
    }

    let query_embedding = crate::retrieval::embedding::embed_query(query_text)?;
    let vector_outcome = crate::retrieval::vector::vector_search_embedding_filtered(
        conn,
        &query_embedding,
        crate::retrieval::vector::VectorSearchFilters {
            project,
            memory_type,
            branch,
            include_stale,
        },
        fetch as usize,
    )?;
    if let Some(reason) = vector_outcome.disabled_reason {
        channels.push(NamedChannel::disabled("vector", weights.vector, reason));
    } else {
        let hits = vector_outcome
            .hits
            .into_iter()
            .filter(|hit| hit.distance <= weights.max_vector_distance)
            .map(|hit| WeightedRankedHit {
                id: hit.memory_id,
                normalized_score: vector_similarity_score(hit.distance, weights),
            })
            .collect();
        channels.push(NamedChannel::enabled_with_hits(
            "vector",
            weights.vector,
            hits,
        ));
    }

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
        let like = memory::search_memories_like_filtered(
            conn,
            &core_refs,
            project,
            memory_type,
            fetch,
            0,
            include_stale,
            branch,
        )?;
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

    Ok(QuerySearchPlan {
        expanded_terms: expanded,
        core_terms: core_tokens,
        claim_terms,
        fts_query,
        temporal_range,
        temporal_field,
        fetch_limit: fetch,
        weights,
        channels,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_explain(
    query_text: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
    plan: &QuerySearchPlan,
    fused: &[(i64, f64)],
    filtered_result_count: usize,
    paged: &[Memory],
) -> SearchExplain {
    let channels = plan
        .channels
        .iter()
        .map(|channel| SearchExplainChannel {
            name: channel.name.to_string(),
            enabled: channel.is_enabled(),
            disabled_reason: channel.disabled_reason.clone(),
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
    let id_to_score: HashMap<i64, f64> = fused.iter().copied().collect();
    let now_epoch = chrono::Utc::now().timestamp();
    let results = paged
        .iter()
        .enumerate()
        .map(|(index, memory)| SearchExplainResult {
            memory_id: memory.id,
            final_rank: index + 1,
            final_score: id_to_score.get(&memory.id).copied().unwrap_or_default(),
            evidence_confidence: candidate_confidence(memory, plan),
            project: memory.project.clone(),
            scope: memory.scope.clone(),
            visibility: visibility_label(memory, project).to_string(),
            staleness: memory::memory_staleness_label(memory, now_epoch),
            contributions: contributions_for(memory.id, plan),
        })
        .collect();
    SearchExplain {
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
        channels,
        results,
        has_more: false,
        raw_fallback_count: 0,
    }
}

fn contributions_for(memory_id: i64, plan: &QuerySearchPlan) -> Vec<ChannelContribution> {
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

fn weighted_channel_inputs(channels: &[NamedChannel]) -> Vec<WeightedRankedChannel<'_>> {
    channels
        .iter()
        .filter(|channel| channel.has_hits())
        .map(|channel| WeightedRankedChannel {
            weight: channel.weight,
            hits: &channel.hits,
        })
        .collect()
}

fn apply_confidence_gate(
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

fn candidate_confidence(memory: &Memory, plan: &QuerySearchPlan) -> f64 {
    if plan.claim_terms.is_empty() || has_only_vector_evidence(memory.id, plan) {
        return 1.0;
    }
    claim_term_coverage(memory, &plan.claim_terms)
}

fn has_only_vector_evidence(memory_id: i64, plan: &QuerySearchPlan) -> bool {
    let contributing: Vec<&str> = plan
        .channels
        .iter()
        .filter(|channel| channel.hits.iter().any(|hit| hit.id == memory_id))
        .map(|channel| channel.name)
        .collect();
    !contributing.is_empty() && contributing.iter().all(|channel| *channel == "vector")
}

fn vector_similarity_score(distance: f32, weights: SearchWeights) -> f64 {
    let threshold = f64::from(weights.max_vector_distance);
    ((threshold - f64::from(distance)) / threshold).clamp(0.0, 1.0)
}

fn fts_normalized_hits(
    hits: &[crate::retrieval::memory_search::FtsMemoryHit],
) -> Vec<WeightedRankedHit> {
    let best = hits
        .iter()
        .map(|hit| hit.score)
        .fold(f64::INFINITY, f64::min);
    let worst = hits
        .iter()
        .map(|hit| hit.score)
        .fold(f64::NEG_INFINITY, f64::max);
    let spread = worst - best;
    hits.iter()
        .enumerate()
        .map(|(rank, hit)| WeightedRankedHit {
            id: hit.memory.id,
            normalized_score: if spread.abs() < f64::EPSILON {
                rank_normalized_score(rank)
            } else {
                ((worst - hit.score) / spread).clamp(0.0, 1.0)
            },
        })
        .collect()
}

fn claim_terms(query_text: &str, core_terms: &[String], project: Option<&str>) -> Vec<String> {
    let entity_terms: HashSet<String> = crate::retrieval::entity::extract_entities("", query_text)
        .into_iter()
        .filter_map(|term| normalize_claim_token(&term))
        .collect();
    let project_terms: HashSet<String> = project
        .into_iter()
        .flat_map(|project| project.split(|c: char| !c.is_alphanumeric() && !is_cjk(c)))
        .filter_map(normalize_claim_token)
        .collect();

    core_terms
        .iter()
        .filter_map(|term| normalize_claim_token(term))
        .filter(|term| !entity_terms.contains(term) && !project_terms.contains(term))
        .collect()
}

fn claim_term_coverage(memory: &Memory, claim_terms: &[String]) -> f64 {
    if claim_terms.is_empty() {
        return 1.0;
    }
    let haystack = format!("{} {}", memory.title, memory.text).to_lowercase();
    let matched = claim_terms
        .iter()
        .filter(|term| claim_term_matches(&haystack, term))
        .count();
    matched as f64 / claim_terms.len() as f64
}

fn claim_term_matches(haystack: &str, term: &str) -> bool {
    if haystack.contains(term) {
        return true;
    }
    if claim_term_aliases(term)
        .iter()
        .any(|alias| haystack.contains(alias))
    {
        return true;
    }
    claim_term_stems(term)
        .iter()
        .any(|stem| stem.chars().count() >= 3 && haystack.contains(stem.as_str()))
}

fn claim_term_aliases(term: &str) -> &'static [&'static str] {
    match term {
        "child" | "children" | "kid" | "kids" => &[
            "child",
            "children",
            "kid",
            "kids",
            "son",
            "daughter",
            "sons",
            "daughters",
        ],
        _ => &[],
    }
}

fn claim_term_stems(term: &str) -> Vec<String> {
    let mut stems = Vec::new();
    if let Some(stem) = term.strip_suffix("ing") {
        stems.push(stem.to_string());
    }
    if let Some(stem) = term.strip_suffix("ed") {
        stems.push(stem.to_string());
        stems.push(format!("{stem}e"));
    }
    if let Some(stem) = term.strip_suffix('s') {
        stems.push(stem.to_string());
    }
    stems
}

fn normalize_claim_token(term: &str) -> Option<String> {
    let normalized = term
        .trim_matches(|c: char| !c.is_alphanumeric() && !is_cjk(c))
        .to_lowercase();
    let min_len = if normalized.chars().any(is_cjk) { 2 } else { 3 };
    if normalized.chars().count() < min_len || is_generic_query_term(&normalized) {
        None
    } else {
        Some(normalized)
    }
}

fn is_generic_query_term(term: &str) -> bool {
    matches!(
        term,
        "all"
            | "and"
            | "are"
            | "did"
            | "does"
            | "for"
            | "from"
            | "current"
            | "had"
            | "has"
            | "have"
            | "handles"
            | "how"
            | "into"
            | "is"
            | "its"
            | "latest"
            | "onto"
            | "project"
            | "show"
            | "that"
            | "the"
            | "this"
            | "through"
            | "today"
            | "tomorrow"
            | "yesterday"
            | "before"
            | "after"
            | "during"
            | "only"
            | "production"
            | "was"
            | "were"
            | "what"
            | "when"
            | "where"
            | "which"
            | "who"
            | "why"
            | "with"
    )
}

fn is_cjk(c: char) -> bool {
    matches!(
        c,
        '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}'
    )
}

fn visibility_label(memory: &Memory, requested_project: Option<&str>) -> &'static str {
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
