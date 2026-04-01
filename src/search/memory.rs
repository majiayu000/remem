use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};

use super::common::{paginate_memories, rrf_fuse, sanitize_fts_query};

pub fn search(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
) -> Result<Vec<Memory>> {
    search_with_branch(
        conn,
        query,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        None,
    )
}

pub fn search_with_branch(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
) -> Result<Vec<Memory>> {
    let page_target = (limit.max(1) + offset.max(0) + 1).max(2);

    match query {
        Some(query_text) if !query_text.is_empty() => {
            let fetch = page_target * 3;
            let expanded = crate::query_expand::expand_query(query_text);
            let exp_strs: Vec<&str> = expanded.iter().map(|token| token.as_str()).collect();
            let long_tokens: Vec<&str> = exp_strs
                .iter()
                .filter(|token| token.chars().count() >= 3)
                .copied()
                .collect();

            let core_tokens = crate::query_expand::core_tokens(query_text);
            let core_refs: Vec<&str> = core_tokens.iter().map(|token| token.as_str()).collect();
            let mut channels: Vec<Vec<i64>> = Vec::new();

            if !long_tokens.is_empty() {
                let safe_query = sanitize_fts_query(&long_tokens.join(" "));
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
                channels.push(fts.iter().map(|memory| memory.id).collect());
            }

            let entity_ids = crate::entity::search_by_entity_filtered(
                conn,
                query_text,
                project,
                memory_type,
                branch,
                fetch,
                include_stale,
            )?;
            if !entity_ids.is_empty() {
                channels.push(entity_ids);
            }

            if let Some(temporal_constraint) = crate::temporal::extract_temporal(query_text) {
                let temporal_ids = crate::temporal::search_by_time_filtered(
                    conn,
                    &temporal_constraint,
                    project,
                    memory_type,
                    branch,
                    fetch,
                    include_stale,
                )?;
                if !temporal_ids.is_empty() {
                    channels.push(temporal_ids);
                }
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
                channels.push(like.iter().map(|memory| memory.id).collect());
            }

            if channels.is_empty() {
                return Ok(vec![]);
            }

            let final_ids: Vec<i64> = rrf_fuse(&channels, 60.0)
                .iter()
                .map(|(id, _)| *id)
                .collect();
            let loaded = memory::get_memories_by_ids(conn, &final_ids, None)?;
            let id_to_memory: HashMap<i64, Memory> = loaded
                .into_iter()
                .map(|memory| (memory.id, memory))
                .collect();
            let ordered: Vec<Memory> = final_ids
                .iter()
                .filter_map(|id| id_to_memory.get(id).cloned())
                .collect();
            Ok(paginate_memories(ordered, limit, offset))
        }
        _ => {
            let project_name = project.unwrap_or("");
            if project_name.is_empty() {
                Ok(vec![])
            } else {
                memory::list_memories(
                    conn,
                    project_name,
                    memory_type,
                    limit,
                    offset,
                    include_stale,
                    branch,
                )
            }
        }
    }
}
