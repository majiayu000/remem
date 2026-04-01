use anyhow::Result;
use rusqlite::Connection;

use crate::db::Observation;

use super::shared::{
    collect_rows, map_observation_row, obs_select_cols, push_project_filter, EPOCH_SECS_ONLY,
};

pub fn search_observations_fts(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    obs_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
) -> Result<Vec<Observation>> {
    let mut conditions = vec![
        "observations_fts MATCH ?1".to_string(),
        format!("o.{EPOCH_SECS_ONLY}"),
    ];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(query.to_string())];

    let mut idx = 2;
    if let Some(project_name) = project {
        let (project_filter, next_idx) =
            push_project_filter("o.project", project_name, idx, &mut param_values);
        conditions.push(project_filter);
        idx = next_idx;
    }
    if let Some(obs_type_name) = obs_type {
        conditions.push(format!("o.type = ?{idx}"));
        param_values.push(Box::new(obs_type_name.to_string()));
        idx += 1;
    }
    if !include_stale {
        conditions.push("o.status = 'active'".to_string());
    }

    param_values.push(Box::new(limit));
    param_values.push(Box::new(offset));

    let sql = format!(
        "SELECT {} \
         FROM observations o \
         JOIN observations_fts ON observations_fts.rowid = o.id \
         WHERE {} \
         ORDER BY (((-rank) / (1.0 + 0.5 * (CASE \
           WHEN (strftime('%s','now') - o.created_at_epoch) > 0 \
             THEN (strftime('%s','now') - o.created_at_epoch) \
           ELSE 0 \
         END) / 2592000.0)) * CASE WHEN o.status = 'stale' THEN 0.25 ELSE 1.0 END) DESC \
         LIMIT ?{} OFFSET ?{}",
        obs_select_cols("o"),
        conditions.join(" AND "),
        idx,
        idx + 1
    );

    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), map_observation_row)?;
    collect_rows(rows)
}

pub fn search_observations_like(
    conn: &Connection,
    tokens: &[&str],
    project: Option<&str>,
    obs_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
) -> Result<Vec<Observation>> {
    if tokens.is_empty() {
        return Ok(vec![]);
    }

    let mut conditions = vec![format!("o.{EPOCH_SECS_ONLY}")];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;

    for token in tokens {
        let like_pattern = format!("%{token}%");
        let cols = [
            "o.title",
            "o.subtitle",
            "o.narrative",
            "o.facts",
            "o.concepts",
        ];
        let token_clauses: Vec<String> = cols
            .iter()
            .map(|col| format!("{col} LIKE ?{idx}"))
            .collect();
        param_values.push(Box::new(like_pattern));
        conditions.push(format!("({})", token_clauses.join(" OR ")));
        idx += 1;
    }

    if let Some(project_name) = project {
        let (project_filter, next_idx) =
            push_project_filter("o.project", project_name, idx, &mut param_values);
        conditions.push(project_filter);
        idx = next_idx;
    }
    if let Some(obs_type_name) = obs_type {
        conditions.push(format!("o.type = ?{idx}"));
        param_values.push(Box::new(obs_type_name.to_string()));
        idx += 1;
    }
    if !include_stale {
        conditions.push("o.status = 'active'".to_string());
    }

    param_values.push(Box::new(limit));
    param_values.push(Box::new(offset));

    let sql = format!(
        "SELECT {} FROM observations o \
         WHERE {} \
         ORDER BY o.created_at_epoch DESC \
         LIMIT ?{} OFFSET ?{}",
        obs_select_cols("o"),
        conditions.join(" AND "),
        idx,
        idx + 1
    );

    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), map_observation_row)?;
    collect_rows(rows)
}
