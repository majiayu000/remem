use std::collections::HashSet;

use anyhow::Result;
use rusqlite::Connection;

use crate::memory;

fn push_unique_ids(target: &mut Vec<i64>, ids: impl IntoIterator<Item = i64>, first_hop_set: &HashSet<i64>) {
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
    fetch: i64,
    first_hop_set: &HashSet<i64>,
) -> Result<Vec<i64>> {
    let mut second_hop_ids = Vec::new();

    for entity_name in discovered_entities {
        let entity_results = crate::entity::search_by_entity(conn, entity_name, project, fetch)?;
        push_unique_ids(&mut second_hop_ids, entity_results, first_hop_set);
    }

    for entity_name in discovered_entities {
        let safe_query = format!("\"{}\"", entity_name.replace('"', "\"\""));
        if let Ok(fts_results) = memory::search_memories_fts(conn, &safe_query, project, None, fetch, 0) {
            push_unique_ids(
                &mut second_hop_ids,
                fts_results.into_iter().map(|memory| memory.id),
                first_hop_set,
            );
        }
    }

    Ok(second_hop_ids)
}
