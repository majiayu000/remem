use anyhow::Result;
use rusqlite::Connection;

use crate::db::Observation;
use crate::db_models::OBSERVATION_TYPES;
use crate::db_query;

use super::common::sanitize_fts_query;

pub fn search_observations(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    obs_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
) -> Result<Vec<Observation>> {
    let mut results = match query {
        Some(query_text) if !query_text.is_empty() => {
            let tokens: Vec<&str> = query_text.split_whitespace().collect();
            let has_short_token = tokens.iter().any(|token| token.chars().count() < 3);

            if has_short_token {
                db_query::search_observations_like(
                    conn,
                    &tokens,
                    project,
                    obs_type,
                    limit,
                    offset,
                    include_stale,
                )?
            } else {
                let safe_query = sanitize_fts_query(query_text);
                db_query::search_observations_fts(
                    conn,
                    &safe_query,
                    project,
                    obs_type,
                    limit,
                    offset,
                    include_stale,
                )?
            }
        }
        _ => {
            let types: Vec<&str> =
                obs_type.map_or_else(|| OBSERVATION_TYPES.to_vec(), |kind| vec![kind]);
            let project_name = project.unwrap_or("");
            if project_name.is_empty() {
                return Ok(vec![]);
            }
            db_query::query_observations(conn, project_name, &types, limit)?
        }
    };

    if let Some(project_name) = project {
        results.retain(|observation| {
            crate::project_id::project_matches(observation.project.as_deref(), project_name)
        });
    }

    let start = offset as usize;
    if start >= results.len() {
        return Ok(vec![]);
    }
    let end = (start + limit as usize).min(results.len());
    Ok(results[start..end].to_vec())
}
