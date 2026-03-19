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

/// Check if a project value matches the filter using suffix matching.
/// "harness" matches "harness", "tools/harness", "vibeguard/harness".
fn project_matches(project: &str, filter: &str) -> bool {
    project == filter || project.ends_with(&format!("/{filter}"))
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
    // Search without project filter in SQL, apply suffix matching post-filter.
    // This handles project fragmentation (e.g., "harness" vs "tools/harness").
    let fetch_limit = if project.is_some() { limit * 3 } else { limit };

    let mut results = match query {
        Some(q) if !q.is_empty() => {
            let tokens: Vec<&str> = q.split_whitespace().collect();
            let has_short_token = tokens.iter().any(|t| t.chars().count() < 3);

            if has_short_token {
                memory::search_memories_like(conn, &tokens, None, memory_type, fetch_limit, 0)?
            } else {
                let safe_query = sanitize_fts_query(q);
                memory::search_memories_fts(conn, &safe_query, None, memory_type, fetch_limit, 0)?
            }
        }
        _ => {
            // No query — return recent memories; need project for this path
            let proj = project.unwrap_or("");
            if proj.is_empty() {
                return Ok(vec![]);
            } else if let Some(t) = memory_type {
                memory::get_memories_by_type(conn, proj, t, fetch_limit)?
            } else {
                memory::get_recent_memories(conn, proj, fetch_limit)?
            }
        }
    };

    // Post-filter by project using suffix matching
    if let Some(proj) = project {
        results.retain(|m| project_matches(&m.project, proj));
    }

    // Apply offset and limit
    let start = offset as usize;
    if start >= results.len() {
        return Ok(vec![]);
    }
    let end = (start + limit as usize).min(results.len());
    Ok(results[start..end].to_vec())
}

/// Search observations (used by get_observations MCP tool).
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

    let fetch_limit = if project.is_some() { limit * 3 } else { limit };

    let mut results = match query {
        Some(q) if !q.is_empty() => {
            let tokens: Vec<&str> = q.split_whitespace().collect();
            let has_short_token = tokens.iter().any(|t| t.chars().count() < 3);

            if has_short_token {
                db_query::search_observations_like(
                    conn,
                    &tokens,
                    None,
                    obs_type,
                    fetch_limit,
                    0,
                    include_stale,
                )?
            } else {
                let safe_query = sanitize_fts_query(q);
                db_query::search_observations_fts(
                    conn,
                    &safe_query,
                    None,
                    obs_type,
                    fetch_limit,
                    0,
                    include_stale,
                )?
            }
        }
        _ => {
            let types: Vec<&str> = obs_type.map_or_else(|| OBSERVATION_TYPES.to_vec(), |t| vec![t]);
            let proj = project.unwrap_or("");
            if proj.is_empty() {
                return Ok(vec![]);
            } else {
                db_query::query_observations(conn, proj, &types, fetch_limit)?
            }
        }
    };

    // Post-filter by project using suffix matching
    if let Some(proj) = project {
        results.retain(|o| {
            o.project
                .as_deref()
                .map_or(false, |p| project_matches(p, proj))
        });
    }

    let start = offset as usize;
    if start >= results.len() {
        return Ok(vec![]);
    }
    let end = (start + limit as usize).min(results.len());
    Ok(results[start..end].to_vec())
}
