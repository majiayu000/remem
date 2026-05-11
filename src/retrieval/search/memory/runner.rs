use anyhow::Result;
use rusqlite::Connection;

use crate::memory::Memory;

use super::listing::search_without_query;
use super::text::search_with_query;

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
