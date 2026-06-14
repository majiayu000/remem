use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};

use super::discover::discover_entities;
use super::expand::collect_second_hop_ids;
use super::merge::rank_merged_scores;
use super::types::MultiHopResult;

fn paginate_memories(memories: Vec<Memory>, limit: i64, offset: i64) -> Vec<Memory> {
    let start = offset.max(0) as usize;
    if start >= memories.len() {
        return vec![];
    }
    let end = (start + limit.max(0) as usize).min(memories.len());
    memories[start..end].to_vec()
}

fn load_ranked_memories(conn: &Connection, ids: &[i64]) -> Result<Vec<Memory>> {
    let loaded = memory::get_memories_by_ids(conn, ids, None)?;
    let id_to_mem: HashMap<i64, Memory> = loaded
        .into_iter()
        .map(|memory| (memory.id, memory))
        .collect();
    Ok(ids
        .iter()
        .filter_map(|id| id_to_mem.get(id).cloned())
        .collect())
}

pub fn search_multi_hop(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    limit: i64,
    offset: i64,
    memory_type: Option<&str>,
    branch: Option<&str>,
    include_stale: bool,
) -> Result<MultiHopResult> {
    let page_target = (limit.max(0) + offset.max(0)).max(1);
    let fetch = page_target * 3;
    let first_hop = crate::retrieval::search::search_with_branch(
        conn,
        Some(query),
        project,
        memory_type,
        fetch,
        0,
        include_stale,
        branch,
    )?;
    let first_hop_ids: Vec<i64> = first_hop.iter().map(|memory| memory.id).collect();

    if first_hop.is_empty() {
        return Ok(MultiHopResult {
            memories: vec![],
            hops: 1,
            entities_discovered: vec![],
        });
    }

    let discovered_entities = discover_entities(query, &first_hop);
    if discovered_entities.is_empty() {
        return Ok(MultiHopResult {
            memories: paginate_memories(first_hop, limit, offset),
            hops: 1,
            entities_discovered: vec![],
        });
    }

    let first_hop_set: HashSet<i64> = first_hop_ids.iter().copied().collect();
    let second_hop_ids = collect_second_hop_ids(
        conn,
        &discovered_entities,
        project,
        memory_type,
        branch,
        include_stale,
        fetch,
        &first_hop_set,
    )?;
    if second_hop_ids.is_empty() {
        return Ok(MultiHopResult {
            memories: paginate_memories(first_hop, limit, offset),
            hops: 1,
            entities_discovered: discovered_entities,
        });
    }

    let merged = rank_merged_scores(&first_hop_ids, &second_hop_ids, fetch);
    let merged_ids = merged.iter().map(|(id, _)| *id).collect::<Vec<_>>();
    let ranked = load_ranked_memories(conn, &merged_ids)?;
    let (ranked, _) = crate::retrieval::search::apply_score_demotions(conn, &merged, ranked)?;
    let memories = paginate_memories(
        ranked.into_iter().take(page_target as usize).collect(),
        limit,
        offset,
    );
    Ok(MultiHopResult {
        memories,
        hops: 2,
        entities_discovered: discovered_entities,
    })
}
