use anyhow::Result;
use rusqlite::Connection;

use crate::db::{self, Observation};
use crate::memory_format::OBSERVATION_TYPES;

/// Escape a user query for FTS5 MATCH safety.
/// Wraps each whitespace-separated token in double quotes so that
/// special characters like `-`, `/`, `.` are treated as literals
/// instead of FTS5 operators.
fn sanitize_fts_query(raw: &str) -> String {
    raw.split_whitespace()
        .map(|token| {
            // Escape any embedded double quotes by doubling them
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
    obs_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
) -> Result<Vec<Observation>> {
    match query {
        Some(q) if !q.is_empty() => {
            let safe_query = sanitize_fts_query(q);
            db::search_observations_fts(
                conn,
                &safe_query,
                project,
                obs_type,
                limit,
                offset,
                include_stale,
            )
        }
        _ => {
            // No query — return recent observations filtered by project/type
            let types: Vec<&str> = obs_type.map_or_else(|| OBSERVATION_TYPES.to_vec(), |t| vec![t]);
            let proj = project.unwrap_or("");
            if proj.is_empty() {
                Ok(vec![])
            } else {
                db::query_observations(conn, proj, &types, limit)
            }
        }
    }
}
