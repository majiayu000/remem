use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};

/// Escape a user query for FTS5 MATCH safety.
/// Wraps each whitespace-separated token in double quotes so that
/// special characters like `-`, `/`, `.` are treated as literals
/// instead of FTS5 operators.
fn sanitize_fts_query(raw: &str) -> String {
    raw.split_whitespace()
        .map(|token| {
            let escaped = token.replace('"', "\"\"");
            format!("\"{escaped}\"")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn search(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    _include_stale: bool,
) -> Result<Vec<Memory>> {
    match query {
        Some(q) if !q.is_empty() => {
            let tokens: Vec<&str> = q.split_whitespace().collect();
            let has_short_token = tokens.iter().any(|t| t.chars().count() < 3);

            if has_short_token {
                memory::search_memories_like(conn, &tokens, project, memory_type, limit, offset)
            } else {
                let safe_query = sanitize_fts_query(q);
                memory::search_memories_fts(conn, &safe_query, project, memory_type, limit, offset)
            }
        }
        _ => {
            // No query — return recent memories filtered by project/type
            let proj = project.unwrap_or("");
            if proj.is_empty() {
                Ok(vec![])
            } else if let Some(t) = memory_type {
                memory::get_memories_by_type(conn, proj, t, limit)
            } else {
                memory::get_recent_memories(conn, proj, limit)
            }
        }
    }
}

/// Search observations (legacy, still used by get_observations MCP tool).
pub fn search_observations(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    obs_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
) -> Result<Vec<crate::db::Observation>> {
    use crate::db_models::OBSERVATION_TYPES;
    use crate::db_query;

    match query {
        Some(q) if !q.is_empty() => {
            let tokens: Vec<&str> = q.split_whitespace().collect();
            let has_short_token = tokens.iter().any(|t| t.chars().count() < 3);

            if has_short_token {
                db_query::search_observations_like(
                    conn,
                    &tokens,
                    project,
                    obs_type,
                    limit,
                    offset,
                    include_stale,
                )
            } else {
                let safe_query = sanitize_fts_query(q);
                db_query::search_observations_fts(
                    conn,
                    &safe_query,
                    project,
                    obs_type,
                    limit,
                    offset,
                    include_stale,
                )
            }
        }
        _ => {
            let types: Vec<&str> = obs_type.map_or_else(|| OBSERVATION_TYPES.to_vec(), |t| vec![t]);
            let proj = project.unwrap_or("");
            if proj.is_empty() {
                Ok(vec![])
            } else {
                db_query::query_observations(conn, proj, &types, limit)
            }
        }
    }
}
