use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};

/// Escape and join tokens with OR for FTS5 MATCH.
fn sanitize_fts_query(raw: &str) -> String {
    let tokens: Vec<String> = raw
        .split_whitespace()
        .map(|token| {
            let escaped = token.replace('"', "\"\"");
            format!("\"{escaped}\"")
        })
        .collect();
    if tokens.len() <= 1 {
        tokens.join("")
    } else {
        tokens.join(" OR ")
    }
}

/// Reciprocal Rank Fusion: merge multiple ranked lists.
/// score(d) = Σ 1/(k + rank_i(d)), k=60 (standard).
fn rrf_fuse(channels: &[Vec<i64>], k: f64) -> Vec<(i64, f64)> {
    let mut scores: HashMap<i64, f64> = HashMap::new();
    for channel in channels {
        for (rank, &id) in channel.iter().enumerate() {
            *scores.entry(id).or_default() += 1.0 / (k + rank as f64 + 1.0);
        }
    }
    let mut results: Vec<_> = scores.into_iter().collect();
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results
}

pub fn search(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    _include_stale: bool,
) -> Result<Vec<Memory>> {
    search_with_branch(conn, query, project, memory_type, limit, offset, _include_stale, None)
}

pub fn search_with_branch(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    _offset: i64,
    _include_stale: bool,
    branch: Option<&str>,
) -> Result<Vec<Memory>> {
    let mut results = match query {
        Some(q) if !q.is_empty() => {
            let fetch = limit * 2; // Over-fetch for RRF fusion
            let orig_tokens: Vec<&str> = q.split_whitespace().collect();
            let expanded = crate::query_expand::expand_query(q);
            let exp_strs: Vec<&str> = expanded.iter().map(|s| s.as_str()).collect();
            let long_tokens: Vec<&str> = exp_strs.iter().filter(|t| t.chars().count() >= 3).copied().collect();

            let mut channels: Vec<Vec<i64>> = Vec::new();

            // Channel 1: FTS5 with expanded OR query
            if !long_tokens.is_empty() {
                let safe_query = sanitize_fts_query(&long_tokens.join(" "));
                let fts = memory::search_memories_fts(conn, &safe_query, project, memory_type, fetch, 0)?;
                channels.push(fts.iter().map(|m| m.id).collect());
            }

            // Channel 2: Entity search
            let entity_ids = crate::entity::search_by_entity(conn, q, fetch)?;
            if !entity_ids.is_empty() {
                channels.push(entity_ids);
            }

            // Channel 3: Temporal search
            if let Some(tc) = crate::temporal::extract_temporal(q) {
                let temporal_ids = crate::temporal::search_by_time(conn, &tc, project, fetch)?;
                if !temporal_ids.is_empty() {
                    channels.push(temporal_ids);
                }
            }

            // Channel 4: LIKE fallback with original tokens
            let like = memory::search_memories_like(conn, &orig_tokens, project, memory_type, fetch, 0)?;
            if !like.is_empty() {
                channels.push(like.iter().map(|m| m.id).collect());
            }

            if channels.is_empty() {
                vec![]
            } else {
                // RRF fusion
                let fused = rrf_fuse(&channels, 60.0);
                let top_ids: Vec<i64> = fused.iter().take(limit as usize).map(|(id, _)| *id).collect();

                // Batch load memories by fused IDs, preserving RRF order
                let loaded = memory::get_memories_by_ids(conn, &top_ids, None)?;
                let id_to_mem: HashMap<i64, Memory> = loaded.into_iter().map(|m| (m.id, m)).collect();
                top_ids.iter().filter_map(|id| id_to_mem.get(id).cloned()).collect()
            }
        }
        _ => {
            let proj = project.unwrap_or("");
            if proj.is_empty() {
                return Ok(vec![]);
            } else if let Some(t) = memory_type {
                memory::get_memories_by_type(conn, proj, t, limit)?
            } else {
                memory::get_recent_memories(conn, proj, limit)?
            }
        }
    };

    // Post-filter by branch
    if let Some(br) = branch {
        results.retain(|m| match &m.branch {
            Some(b) => b == br,
            None => true,
        });
    }

    Ok(results)
}

/// Search observations (used by get_observations MCP tool).
pub fn search_observations(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    obs_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
) -> Result<Vec<crate::db::Observation>> {
    use crate::db_models::OBSERVATION_TYPES;
    use crate::db_query;

    let mut results = match query {
        Some(q) if !q.is_empty() => {
            let tokens: Vec<&str> = q.split_whitespace().collect();
            let has_short_token = tokens.iter().any(|t| t.chars().count() < 3);

            if has_short_token {
                db_query::search_observations_like(conn, &tokens, project, obs_type, limit, offset, include_stale)?
            } else {
                let safe_query = sanitize_fts_query(q);
                db_query::search_observations_fts(conn, &safe_query, project, obs_type, limit, offset, include_stale)?
            }
        }
        _ => {
            let types: Vec<&str> = obs_type.map_or_else(|| OBSERVATION_TYPES.to_vec(), |t| vec![t]);
            let proj = project.unwrap_or("");
            if proj.is_empty() {
                return Ok(vec![]);
            } else {
                db_query::query_observations(conn, proj, &types, limit)?
            }
        }
    };

    if let Some(proj) = project {
        results.retain(|o| {
            o.project.as_deref().is_some_and(|p| p == proj || p.ends_with(&format!("/{proj}")))
        });
    }

    let start = offset as usize;
    if start >= results.len() {
        return Ok(vec![]);
    }
    let end = (start + limit as usize).min(results.len());
    Ok(results[start..end].to_vec())
}
