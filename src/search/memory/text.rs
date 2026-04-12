use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};

use super::super::common::{paginate_memories, rrf_fuse, sanitize_fts_query};

const LIKE_SEPARATORS: [char; 4] = ['-', '_', '/', '.'];

fn load_ordered_memories(conn: &Connection, ids: &[i64]) -> Result<Vec<Memory>> {
    let loaded = memory::get_memories_by_ids(conn, ids, None)?;
    let mut id_to_memory: HashMap<i64, Memory> = HashMap::with_capacity(loaded.len());
    for memory in loaded {
        id_to_memory.insert(memory.id, memory);
    }

    let mut ordered = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(memory) = id_to_memory.remove(id) {
            ordered.push(memory);
        }
    }
    Ok(ordered)
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
    let page_target = (limit.max(1) + offset.max(0) + 1).max(2);
    let fetch = page_target * 3;
    let expanded = crate::query_expand::expand_query(query_text);
    let expanded_refs: Vec<&str> = expanded.iter().map(|token| token.as_str()).collect();
    let long_tokens: Vec<&str> = expanded_refs
        .iter()
        .filter(|token| token.chars().count() >= 3)
        .copied()
        .collect();

    let core_tokens = crate::query_expand::core_tokens(query_text);
    let core_refs: Vec<&str> = core_tokens.iter().map(|token| token.as_str()).collect();
    let has_short_core_token = core_refs.iter().any(|token| token.chars().count() < 3);
    let has_separator_core_token = core_refs
        .iter()
        .any(|token| token.chars().any(|ch| LIKE_SEPARATORS.contains(&ch)));
    let separator_like_tokens = split_separator_like_tokens(&core_refs);
    let like_tokens = if separator_like_tokens.is_empty() {
        core_tokens.clone()
    } else {
        separator_like_tokens
    };
    let like_refs: Vec<&str> = like_tokens.iter().map(|token| token.as_str()).collect();
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
        if !fts.is_empty() {
            channels.push(fts.iter().map(|memory| memory.id).collect());
        }
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

    // LIKE fallback is expensive; reserve it for short-token queries or when
    // all higher-signal channels return empty.
    if (has_short_core_token || has_separator_core_token || channels.is_empty())
        && !like_refs.is_empty()
    {
        let like = memory::search_memories_like_filtered(
            conn,
            &like_refs,
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
    }

    if channels.is_empty() {
        return Ok(vec![]);
    }

    let final_ids: Vec<i64> = rrf_fuse(&channels, 60.0)
        .iter()
        .map(|(id, _)| *id)
        .collect();
    let ordered = load_ordered_memories(conn, &final_ids)?;
    Ok(paginate_memories(ordered, limit, offset))
}

fn split_separator_like_tokens(core_refs: &[&str]) -> Vec<String> {
    use std::collections::HashSet;

    let mut tokens = Vec::new();
    let mut seen = HashSet::new();

    for token in core_refs {
        if !token.chars().any(|ch| LIKE_SEPARATORS.contains(&ch)) {
            continue;
        }
        for part in token.split(|ch| LIKE_SEPARATORS.contains(&ch)) {
            if part.is_empty() {
                continue;
            }
            let lowered = part.to_lowercase();
            if seen.insert(lowered) {
                tokens.push(part.to_string());
            }
        }
    }

    tokens
}
