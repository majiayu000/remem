use std::collections::HashSet;

use anyhow::Result;
use rusqlite::Connection;

use crate::memory;

fn push_unique_ids(
    target: &mut Vec<i64>,
    ids: impl IntoIterator<Item = i64>,
    first_hop_set: &HashSet<i64>,
) {
    for id in ids {
        if !first_hop_set.contains(&id) && !target.contains(&id) {
            target.push(id);
        }
    }
}

pub(crate) fn collect_second_hop_ids(
    conn: &Connection,
    discovered_entities: &[String],
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    include_stale: bool,
    include_suppressed: bool,
    fetch: i64,
    first_hop_set: &HashSet<i64>,
) -> Result<Vec<i64>> {
    let mut second_hop_ids = Vec::new();

    for entity_name in discovered_entities {
        let entity_results = crate::retrieval::entity::search_by_entity_filtered(
            conn,
            entity_name,
            project,
            memory_type,
            branch,
            fetch,
            include_stale,
        )?;
        let entity_results = filter_suppressed_ids(conn, entity_results, include_suppressed)?;
        push_unique_ids(&mut second_hop_ids, entity_results, first_hop_set);
    }

    for entity_name in discovered_entities {
        let safe_query = format!("\"{}\"", entity_name.replace('"', "\"\""));
        match memory::search_memories_fts_filtered(
            conn,
            &safe_query,
            project,
            memory_type,
            fetch,
            0,
            include_stale,
            branch,
        ) {
            Ok(fts_results) => {
                let ids = fts_results.into_iter().map(|memory| memory.id).collect();
                let ids = filter_suppressed_ids(conn, ids, include_suppressed)?;
                push_unique_ids(&mut second_hop_ids, ids, first_hop_set);
            }
            Err(error) => {
                crate::log::warn(
                    "search_multihop",
                    &format!("second-hop FTS fallback failed: {}", error),
                );
            }
        }
    }

    Ok(second_hop_ids)
}

fn filter_suppressed_ids(
    conn: &Connection,
    ids: Vec<i64>,
    include_suppressed: bool,
) -> Result<Vec<i64>> {
    if include_suppressed || ids.is_empty() {
        return Ok(ids);
    }
    let suppressed = crate::memory::suppression::active_suppressed_memory_ids(conn, &ids)?;
    Ok(ids
        .into_iter()
        .filter(|id| !suppressed.contains(id))
        .collect())
}
