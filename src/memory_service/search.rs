use anyhow::Result;
use rusqlite::Connection;

use super::types::{MultiHopMeta, SearchRequest, SearchResultSet};

/// Curated hits below this count trigger a raw archive fallback so the caller
/// always has *something* to show when the conversation happened but was
/// never promoted.
const RAW_FALLBACK_THRESHOLD: usize = 3;
const RAW_FALLBACK_LIMIT: i64 = 10;

pub fn search_memories(conn: &Connection, req: &SearchRequest) -> Result<SearchResultSet> {
    let limit = req.limit.max(1);
    let query = req.query.as_deref();

    if req.multi_hop {
        return multi_hop_search(conn, query, req.project.as_deref(), limit, req);
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
    let raw_hits = maybe_fallback_raw(conn, req, memories.len());
    Ok(SearchResultSet {
        memories,
        multi_hop: None,
        has_more,
        raw_hits,
    })
}

fn multi_hop_search(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    limit: i64,
    req: &SearchRequest,
) -> Result<SearchResultSet> {
    if let Some(query_text) = query.filter(|query_text| !query_text.is_empty()) {
        let mut result =
            crate::search_multihop::search_multi_hop(conn, query_text, project, limit + 1)?;
        let has_more = result.memories.len() as i64 > limit;
        result.memories.truncate(limit as usize);
        let raw_hits = maybe_fallback_raw(conn, req, result.memories.len());
        Ok(SearchResultSet {
            memories: result.memories,
            multi_hop: Some(MultiHopMeta {
                hops: result.hops,
                entities_discovered: result.entities_discovered,
            }),
            has_more,
            raw_hits,
        })
    } else {
        Ok(SearchResultSet {
            memories: vec![],
            multi_hop: Some(MultiHopMeta {
                hops: 1,
                entities_discovered: vec![],
            }),
            has_more: false,
            raw_hits: vec![],
        })
    }
}

fn maybe_fallback_raw(
    conn: &Connection,
    req: &SearchRequest,
    curated_len: usize,
) -> Vec<crate::raw_archive::RawMessage> {
    if curated_len >= RAW_FALLBACK_THRESHOLD {
        return vec![];
    }
    let Some(query) = req
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
    else {
        return vec![];
    };
    let raw_req = crate::raw_archive::RawSearchRequest {
        query: query.to_string(),
        project: req.project.clone(),
        role: None,
        limit: RAW_FALLBACK_LIMIT,
        offset: 0,
    };
    match crate::raw_archive::search_raw_messages(conn, &raw_req) {
        Ok(hits) => hits,
        Err(error) => {
            crate::log::warn("search", &format!("raw archive fallback failed: {}", error));
            vec![]
        }
    }
}
