use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};

use super::super::common::{paginate_memories, rrf_fuse, sanitize_fts_query};
use super::{
    ChannelContribution, ChannelHit, SearchExplain, SearchExplainChannel, SearchExplainResult,
};

const RRF_K: f64 = 60.0;
const MAX_VECTOR_DISTANCE: f32 = 0.51;

pub(super) struct QuerySearchWithExplain {
    pub memories: Vec<Memory>,
    pub explain: SearchExplain,
}

struct QuerySearchPlan {
    expanded_terms: Vec<String>,
    core_terms: Vec<String>,
    fts_query: Option<String>,
    temporal_range: Option<(i64, i64)>,
    temporal_field: Option<String>,
    fetch_limit: i64,
    channels: Vec<NamedChannel>,
}

struct NamedChannel {
    name: &'static str,
    disabled_reason: Option<String>,
    ids: Vec<i64>,
}

impl NamedChannel {
    fn enabled(name: &'static str, ids: Vec<i64>) -> Self {
        Self {
            name,
            disabled_reason: None,
            ids,
        }
    }

    fn disabled(name: &'static str, reason: impl Into<String>) -> Self {
        Self {
            name,
            disabled_reason: Some(reason.into()),
            ids: vec![],
        }
    }

    fn is_enabled(&self) -> bool {
        self.disabled_reason.is_none()
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
    let plan = build_query_search_plan(
        conn,
        query_text,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
    )?;
    if plan.channels.is_empty() {
        return Ok(vec![]);
    }

    let channel_ids: Vec<Vec<i64>> = plan
        .channels
        .iter()
        .map(|channel| channel.ids.clone())
        .collect();
    let final_ids: Vec<i64> = rrf_fuse(&channel_ids, RRF_K)
        .iter()
        .map(|(id, _)| *id)
        .collect();
    let ordered = load_ordered_memories(conn, &final_ids)?;
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
                fts_query: plan.fts_query,
                temporal_range: plan.temporal_range,
                temporal_field: plan.temporal_field,
                rrf_k: RRF_K,
                channels: vec![],
                results: vec![],
                has_more: false,
                raw_fallback_count: 0,
            },
        });
    }

    let channel_ids: Vec<Vec<i64>> = plan
        .channels
        .iter()
        .map(|channel| channel.ids.clone())
        .collect();
    let fused = rrf_fuse(&channel_ids, RRF_K);
    let final_ids: Vec<i64> = fused.iter().map(|(id, _)| *id).collect();
    let ordered = load_ordered_memories(conn, &final_ids)?;
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
        &fused,
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
    let core_refs: Vec<&str> = core_tokens.iter().map(|token| token.as_str()).collect();
    let mut channels: Vec<NamedChannel> = Vec::new();
    let mut fts_query = None;
    let mut temporal_range = None;
    let mut temporal_field = None;

    if !long_tokens.is_empty() {
        let safe_query = sanitize_fts_query(&long_tokens.join(" "));
        fts_query = Some(safe_query.clone());
        let fts = memory::search_memories_fts_filtered(
            conn,
            &safe_query,
            project,
            memory_type,
            fetch,
            0,
            include_stale,
            branch,
        )?;
        let ids: Vec<i64> = fts.iter().map(|memory| memory.id).collect();
        if !ids.is_empty() {
            channels.push(NamedChannel::enabled("fts", ids));
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
        channels.push(NamedChannel::enabled("entity", entity_ids));
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
            channels.push(NamedChannel::enabled("temporal", temporal_ids));
        }
    }

    let query_embedding = crate::retrieval::vector::embed_query_text(query_text);
    let vector_outcome = crate::retrieval::vector::vector_search_filtered(
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
        channels.push(NamedChannel::disabled("vector", reason));
    } else {
        channels.push(NamedChannel::enabled(
            "vector",
            vector_outcome
                .hits
                .into_iter()
                .filter(|hit| hit.distance <= MAX_VECTOR_DISTANCE)
                .map(|hit| hit.memory_id)
                .collect(),
        ));
    }

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
    if !like.is_empty() {
        channels.push(NamedChannel::enabled(
            "like_fallback",
            like.iter().map(|memory| memory.id).collect(),
        ));
    }

    Ok(QuerySearchPlan {
        expanded_terms: expanded,
        core_terms: core_tokens,
        fts_query,
        temporal_range,
        temporal_field,
        fetch_limit: fetch,
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
                .ids
                .iter()
                .enumerate()
                .map(|(index, id)| ChannelHit {
                    memory_id: *id,
                    rank: index + 1,
                })
                .collect(),
        })
        .collect();
    let id_to_score: HashMap<i64, f64> = fused.iter().copied().collect();
    let results = paged
        .iter()
        .enumerate()
        .map(|(index, memory)| SearchExplainResult {
            memory_id: memory.id,
            final_rank: index + 1,
            final_score: id_to_score.get(&memory.id).copied().unwrap_or_default(),
            project: memory.project.clone(),
            scope: memory.scope.clone(),
            visibility: visibility_label(memory, project).to_string(),
            contributions: contributions_for(memory.id, &plan.channels),
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
        fts_query: plan.fts_query.clone(),
        temporal_range: plan.temporal_range,
        temporal_field: plan.temporal_field.clone(),
        rrf_k: RRF_K,
        channels,
        results,
        has_more: false,
        raw_fallback_count: 0,
    }
}

fn contributions_for(memory_id: i64, channels: &[NamedChannel]) -> Vec<ChannelContribution> {
    channels
        .iter()
        .filter_map(|channel| {
            channel
                .ids
                .iter()
                .position(|id| *id == memory_id)
                .map(|index| ChannelContribution {
                    channel: channel.name.to_string(),
                    rank: index + 1,
                    score: 1.0 / (RRF_K + index as f64 + 1.0),
                })
        })
        .collect()
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
