use anyhow::Result;
use rusqlite::Connection;

use super::types::{MultiHopMeta, SearchRequest, SearchResultSet};

pub fn search_memories(conn: &Connection, req: &SearchRequest) -> Result<SearchResultSet> {
    let limit = req.limit.max(1);
    let query = req.query.as_deref();

    if req.multi_hop {
        return multi_hop_search(conn, query, req.project.as_deref(), limit);
    }

    let mut memories = crate::search::search_with_branch(
        conn,
        query,
        req.project.as_deref(),
        req.memory_type.as_deref(),
        limit + 1,
        req.offset.max(0),
        req.include_stale,
        req.branch.as_deref(),
    )?;
    let has_more = memories.len() as i64 > limit;
    memories.truncate(limit as usize);
    Ok(SearchResultSet {
        memories,
        multi_hop: None,
        has_more,
    })
}

fn multi_hop_search(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    limit: i64,
) -> Result<SearchResultSet> {
    if let Some(query_text) = query.filter(|query_text| !query_text.is_empty()) {
        let mut result =
            crate::search_multihop::search_multi_hop(conn, query_text, project, limit + 1)?;
        let has_more = result.memories.len() as i64 > limit;
        result.memories.truncate(limit as usize);
        Ok(SearchResultSet {
            memories: result.memories,
            multi_hop: Some(MultiHopMeta {
                hops: result.hops,
                entities_discovered: result.entities_discovered,
            }),
            has_more,
        })
    } else {
        Ok(SearchResultSet {
            memories: vec![],
            multi_hop: Some(MultiHopMeta {
                hops: 1,
                entities_discovered: vec![],
            }),
            has_more: false,
        })
    }
}
