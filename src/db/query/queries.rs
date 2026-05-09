use anyhow::Result;
use rusqlite::Connection;

use crate::db::Observation;

use super::shared::{
    collect_rows, map_observation_row, obs_select_cols, push_project_filter, EPOCH_SECS_ONLY,
};

pub fn query_observations(
    conn: &Connection,
    project: &str,
    types: &[&str],
    limit: i64,
) -> Result<Vec<Observation>> {
    if types.is_empty() {
        return Ok(vec![]);
    }

    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (project_filter, mut idx) = push_project_filter("project", project, 1, &mut param_values);

    let placeholders: Vec<String> = types
        .iter()
        .map(|_| {
            let placeholder = format!("?{idx}");
            idx += 1;
            placeholder
        })
        .collect();
    for obs_type in types {
        param_values.push(Box::new(obs_type.to_string()));
    }
    param_values.push(Box::new(limit));

    let sql = format!(
        "SELECT {} FROM observations \
         WHERE {} AND {} AND type IN ({}) \
         ORDER BY created_at_epoch DESC LIMIT ?{}",
        obs_select_cols("observations"),
        project_filter,
        EPOCH_SECS_ONLY,
        placeholders.join(", "),
        idx
    );

    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), map_observation_row)?;
    collect_rows(rows)
}

pub fn get_observations_by_ids(
    conn: &Connection,
    ids: &[i64],
    project: Option<&str>,
) -> Result<Vec<Observation>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }

    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let mut conditions = vec![
        format!("id IN ({})", placeholders.join(", ")),
        EPOCH_SECS_ONLY.to_string(),
    ];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();

    if let Some(project_name) = project {
        let (project_filter, _) =
            push_project_filter("project", project_name, ids.len() + 1, &mut param_values);
        conditions.push(project_filter);
    }

    let sql = format!(
        "SELECT {} FROM observations WHERE {} \
         ORDER BY created_at_epoch DESC",
        obs_select_cols("observations"),
        conditions.join(" AND ")
    );

    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), map_observation_row)?;
    collect_rows(rows)
}

pub fn count_active_observations(conn: &Connection, project: &str) -> Result<i64> {
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (project_filter, _) = push_project_filter("project", project, 1, &mut param_values);
    let sql = format!(
        "SELECT COUNT(*) FROM observations \
         WHERE {} AND {} AND status IN ('active', 'stale')",
        project_filter, EPOCH_SECS_ONLY
    );
    let refs = crate::db::to_sql_refs(&param_values);
    let count: i64 = conn.query_row(&sql, refs.as_slice(), |row| row.get(0))?;
    Ok(count)
}

pub fn get_oldest_observations(
    conn: &Connection,
    project: &str,
    keep: i64,
    batch_size: i64,
) -> Result<Vec<Observation>> {
    let total = count_active_observations(conn, project)?;
    let compressible = total - keep;
    if compressible <= 0 {
        return Ok(vec![]);
    }
    let take = compressible.min(batch_size);

    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (project_filter, idx) = push_project_filter("project", project, 1, &mut param_values);
    param_values.push(Box::new(take));

    let sql = format!(
        "SELECT {} FROM observations \
         WHERE {} AND {} AND status IN ('active', 'stale') \
         ORDER BY created_at_epoch ASC LIMIT ?{}",
        obs_select_cols("observations"),
        project_filter,
        EPOCH_SECS_ONLY,
        idx
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), map_observation_row)?;
    collect_rows(rows)
}
