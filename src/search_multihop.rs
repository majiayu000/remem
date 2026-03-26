use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};

/// Multi-hop search: iteratively expand search results by extracting entities
/// from found memories and using them as new search terms.
///
/// Algorithm:
/// 1. Run standard search with original query → first-hop results
/// 2. Extract entities from first-hop results
/// 3. For each extracted entity, run entity search → second-hop results
/// 4. Merge all results with RRF, boosting first-hop results
///
/// This handles questions like:
/// - "What do Melanie's kids like?" → finds "Melanie" memories → extracts "Tom", "Sarah" → finds their preferences
/// - "What events has Caroline participated in?" → finds "Caroline" memories → extracts event names → finds event details
pub fn search_multi_hop(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    limit: i64,
) -> Result<MultiHopResult> {
    let fetch = limit * 3;

    // === Hop 1: Standard search ===
    let first_hop = crate::search::search(conn, Some(query), project, None, fetch, 0, true)?;
    let first_hop_ids: Vec<i64> = first_hop.iter().map(|m| m.id).collect();

    if first_hop.is_empty() {
        return Ok(MultiHopResult {
            memories: vec![],
            hops: 1,
            entities_discovered: vec![],
        });
    }

    // === Entity extraction from first-hop results ===
    // Collect all entities mentioned in first-hop memories
    let mut discovered_entities: Vec<String> = Vec::new();
    let mut seen_entities: HashSet<String> = HashSet::new();

    // Also collect entities from the query itself to avoid re-searching them
    let query_entities = crate::entity::extract_entities(query, "");
    for e in &query_entities {
        seen_entities.insert(e.to_lowercase());
    }

    for mem in &first_hop {
        let entities = crate::entity::extract_entities(&mem.title, &mem.text);
        for e in entities {
            let lower = e.to_lowercase();
            if !seen_entities.contains(&lower) {
                seen_entities.insert(lower);
                discovered_entities.push(e);
            }
        }
    }

    if discovered_entities.is_empty() {
        // No new entities to expand — return first-hop results
        let memories = first_hop.into_iter().take(limit as usize).collect();
        return Ok(MultiHopResult {
            memories,
            hops: 1,
            entities_discovered: vec![],
        });
    }

    // === Hop 2: Search by discovered entities ===
    let first_hop_set: HashSet<i64> = first_hop_ids.iter().copied().collect();
    let mut second_hop_ids: Vec<i64> = Vec::new();

    for entity_name in &discovered_entities {
        let entity_results = crate::entity::search_by_entity(conn, entity_name, project, fetch)?;
        for id in entity_results {
            if !first_hop_set.contains(&id) && !second_hop_ids.contains(&id) {
                second_hop_ids.push(id);
            }
        }
    }

    // Also do FTS search for discovered entity names to catch mentions
    // that weren't linked via the entity table
    for entity_name in &discovered_entities {
        let safe_query = format!("\"{}\"", entity_name.replace('"', "\"\""));
        if let Ok(fts_results) =
            memory::search_memories_fts(conn, &safe_query, project, None, fetch, 0)
        {
            for mem in fts_results {
                if !first_hop_set.contains(&mem.id) && !second_hop_ids.contains(&mem.id) {
                    second_hop_ids.push(mem.id);
                }
            }
        }
    }

    if second_hop_ids.is_empty() {
        let memories = first_hop.into_iter().take(limit as usize).collect();
        return Ok(MultiHopResult {
            memories,
            hops: 1,
            entities_discovered: discovered_entities,
        });
    }

    // === Merge with RRF: first-hop gets higher weight ===
    let mut scores: HashMap<i64, f64> = HashMap::new();
    let k = 60.0;

    // First-hop results: full RRF weight
    for (rank, id) in first_hop_ids.iter().enumerate() {
        *scores.entry(*id).or_default() += 1.0 / (k + rank as f64 + 1.0);
    }

    // Second-hop results: reduced weight (0.5x) since they're indirect
    for (rank, id) in second_hop_ids.iter().enumerate() {
        *scores.entry(*id).or_default() += 0.5 / (k + rank as f64 + 1.0);
    }

    let mut ranked: Vec<(i64, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let top_ids: Vec<i64> = ranked
        .iter()
        .take(limit as usize)
        .map(|(id, _)| *id)
        .collect();
    let loaded = memory::get_memories_by_ids(conn, &top_ids, None)?;
    let id_to_mem: HashMap<i64, Memory> = loaded.into_iter().map(|m| (m.id, m)).collect();
    let memories = top_ids
        .iter()
        .filter_map(|id| id_to_mem.get(id).cloned())
        .collect();

    Ok(MultiHopResult {
        memories,
        hops: 2,
        entities_discovered: discovered_entities,
    })
}

pub struct MultiHopResult {
    pub memories: Vec<Memory>,
    pub hops: u8,
    pub entities_discovered: Vec<String>,
}
