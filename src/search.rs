use anyhow::Result;
use rusqlite::Connection;

use crate::db::{self, Observation};
use crate::memory_format::OBSERVATION_TYPES;

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
            db::search_observations_fts(conn, q, project, obs_type, limit, offset, include_stale)
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
