use super::*;
use crate::memory::Memory;
use crate::retrieval::search::common::{weighted_ranked_fuse, WeightedRankedChannel};

pub(in crate::context) fn query_hybrid_context_memories(
    conn: &Connection,
    project: &str,
    query: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    limit: i64,
    allow_remote_embedding: bool,
) -> Result<Vec<Memory>> {
    query_hybrid_context_memories_page(
        conn,
        project,
        query,
        current_branch,
        excluded_types,
        0,
        limit,
        allow_remote_embedding,
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::context) fn query_hybrid_context_memories_page(
    conn: &Connection,
    project: &str,
    query: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    offset: i64,
    limit: i64,
    allow_remote_embedding: bool,
) -> Result<Vec<Memory>> {
    query_hybrid_context_memories_with_weights_page(
        conn,
        project,
        query,
        current_branch,
        excluded_types,
        offset,
        limit,
        SearchWeights::production(),
        allow_remote_embedding,
    )
}

/// Injection retrieval against explicit weights. The production caller uses
/// calibrated defaults plus the GH-947 usage-weight override.
pub(in crate::context) fn query_hybrid_context_memories_with_weights(
    conn: &Connection,
    project: &str,
    query: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    limit: i64,
    weights: SearchWeights,
    allow_remote_embedding: bool,
) -> Result<Vec<Memory>> {
    query_hybrid_context_memories_with_weights_page(
        conn,
        project,
        query,
        current_branch,
        excluded_types,
        0,
        limit,
        weights,
        allow_remote_embedding,
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::context) fn query_hybrid_context_memories_with_weights_page(
    conn: &Connection,
    project: &str,
    query: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    offset: i64,
    limit: i64,
    weights: SearchWeights,
    allow_remote_embedding: bool,
) -> Result<Vec<Memory>> {
    query_hybrid_context_memories_with_rank_signal_mode_page(
        conn,
        project,
        query,
        current_branch,
        excluded_types,
        offset,
        limit,
        weights,
        InjectionRankSignalMode::PureRrf,
        allow_remote_embedding,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn query_hybrid_context_memories_with_rank_signal_mode(
    conn: &Connection,
    project: &str,
    query: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    limit: i64,
    weights: SearchWeights,
    rank_signal_mode: InjectionRankSignalMode,
    allow_remote_embedding: bool,
) -> Result<Vec<Memory>> {
    query_hybrid_context_memories_with_rank_signal_mode_page(
        conn,
        project,
        query,
        current_branch,
        excluded_types,
        0,
        limit,
        weights,
        rank_signal_mode,
        allow_remote_embedding,
    )
}

#[allow(clippy::too_many_arguments)]
fn query_hybrid_context_memories_with_rank_signal_mode_page(
    conn: &Connection,
    project: &str,
    query: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    offset: i64,
    limit: i64,
    weights: SearchWeights,
    rank_signal_mode: InjectionRankSignalMode,
    allow_remote_embedding: bool,
) -> Result<Vec<Memory>> {
    weights.validate()?;
    if offset < 0 || limit <= 0 || query.trim().is_empty() {
        return Ok(vec![]);
    }
    let end = offset
        .checked_add(limit)
        .ok_or_else(|| anyhow::anyhow!("hybrid ranked page overflow"))?;
    let fetch_limit = end.saturating_mul(3).max(MIN_HYBRID_FETCH_LIMIT);
    let mut channels = Vec::new();
    push_channel(
        &mut channels,
        weights.fts,
        query_local_fts_channel(
            conn,
            project,
            query,
            current_branch,
            excluded_types,
            fetch_limit,
        )?,
    );
    push_channel(
        &mut channels,
        weights.entity,
        query_local_entity_channel(
            conn,
            project,
            query,
            current_branch,
            excluded_types,
            fetch_limit,
        )?,
    );
    push_channel(
        &mut channels,
        weights.temporal,
        query_local_temporal_channel(
            conn,
            project,
            query,
            current_branch,
            excluded_types,
            fetch_limit,
        )?,
    );
    push_channel(
        &mut channels,
        weights.fact,
        query_local_fact_channel(
            conn,
            project,
            query,
            current_branch,
            excluded_types,
            fetch_limit,
        )?,
    );
    push_channel(
        &mut channels,
        weights.vector,
        query_local_vector_channel(
            conn,
            project,
            query,
            current_branch,
            excluded_types,
            weights.max_vector_distance,
            allow_remote_embedding,
        )?,
    );
    if channels.is_empty() {
        push_channel(
            &mut channels,
            weights.like_fallback,
            query_local_like_channel(
                conn,
                project,
                query,
                current_branch,
                excluded_types,
                fetch_limit,
            )?,
        );
    }
    if channels.is_empty() {
        return Ok(vec![]);
    }
    usage::push_usage_channel(conn, &mut channels, weights)?;
    if rank_signal_mode == InjectionRankSignalMode::LegacyRankPseudoScore {
        for channel in &mut channels {
            for (rank, hit) in channel.hits.iter_mut().enumerate() {
                if hit.normalized_score.is_none() {
                    hit.normalized_score = Some(1.0 / (rank as f64 + 1.0));
                }
            }
        }
    }
    let channel_inputs = channels
        .iter()
        .map(|channel| WeightedRankedChannel {
            weight: channel.weight,
            hits: &channel.hits,
        })
        .collect::<Vec<_>>();
    let ids = weighted_ranked_fuse(&channel_inputs, weights.rrf_k)?
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    query_owner_included_memories_by_ids(conn, project, &ids, current_branch, excluded_types)
}

fn push_channel(channels: &mut Vec<ContextChannel>, weight: f64, hits: Vec<WeightedRankedHit>) {
    if !hits.is_empty() {
        channels.push(ContextChannel { weight, hits });
    }
}

pub(in crate::context) fn query_owner_included_memories_by_ids(
    conn: &Connection,
    project: &str,
    ids: &[i64],
    current_branch: Option<&str>,
    excluded_types: &[&str],
) -> Result<Vec<Memory>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let ids_json = serde_json::to_string(ids)?;
    let mut conditions = vec!["id IN (SELECT value FROM json_each(?1))".to_string()];
    let mut params = vec![Box::new(ids_json) as Box<dyn rusqlite::types::ToSql>];
    let mut idx = 2;
    push_context_memory_filters(
        conn,
        project,
        current_branch,
        excluded_types,
        "",
        &mut idx,
        &mut conditions,
        &mut params,
    )?;
    let sql = format!(
        "SELECT {} FROM memories WHERE {}",
        memory::MEMORY_COLS,
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), memory::map_memory_row_pub)?;
    let mut rows_by_id = crate::db::query::collect_rows(rows)?
        .into_iter()
        .map(|memory| (memory.id, memory))
        .collect::<std::collections::HashMap<_, _>>();
    Ok(ids.iter().filter_map(|id| rows_by_id.remove(id)).collect())
}
