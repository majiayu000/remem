use anyhow::Result;
use rusqlite::Connection;

use crate::memory::Memory;

use super::listing::search_without_query;
pub(crate) use super::text::SearchWeights;
use super::text::{search_with_query, search_with_query_explain, search_with_query_weights};
use super::SearchExplain;

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
    match query {
        Some(query_text) if !query_text.is_empty() => search_with_query(
            conn,
            query_text,
            project,
            memory_type,
            limit,
            offset,
            include_stale,
            branch,
        ),
        _ => search_without_query(
            conn,
            project,
            memory_type,
            limit,
            offset,
            include_stale,
            branch,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn search_with_branch_weights(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
    weights: SearchWeights,
) -> Result<Vec<Memory>> {
    match query {
        Some(query_text) if !query_text.is_empty() => search_with_query_weights(
            conn,
            query_text,
            project,
            memory_type,
            limit,
            offset,
            include_stale,
            branch,
            weights,
        ),
        _ => search_without_query(
            conn,
            project,
            memory_type,
            limit,
            offset,
            include_stale,
            branch,
        ),
    }
}

pub fn search_with_branch_explain(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
) -> Result<(Vec<Memory>, Option<SearchExplain>)> {
    match query {
        Some(query_text) if !query_text.is_empty() => search_with_query_explain(
            conn,
            query_text,
            project,
            memory_type,
            limit,
            offset,
            include_stale,
            branch,
        )
        .map(|result| (result.memories, Some(result.explain))),
        _ => Ok((
            search_without_query(
                conn,
                project,
                memory_type,
                limit,
                offset,
                include_stale,
                branch,
            )?,
            None,
        )),
    }
}
