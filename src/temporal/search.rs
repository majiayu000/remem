use anyhow::Result;
use rusqlite::{types::ToSql, Connection};

use crate::temporal::types::TemporalConstraint;

/// Search memories within a time range, sorted by recency.
pub fn search_by_time(
    conn: &Connection,
    constraint: &TemporalConstraint,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<i64>> {
    search_by_time_filtered(conn, constraint, project, None, None, limit, false)
}

pub fn search_by_time_filtered(
    conn: &Connection,
    constraint: &TemporalConstraint,
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    limit: i64,
    include_inactive: bool,
) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    let mut conditions = vec!["updated_at_epoch BETWEEN ?1 AND ?2".to_string()];
    let mut params_vec: Vec<Box<dyn ToSql>> = vec![
        Box::new(constraint.start_epoch),
        Box::new(constraint.end_epoch),
    ];
    let mut idx = 3;

    if !include_inactive {
        conditions.push("status = 'active'".to_string());
    }
    if let Some(project) = project {
        conditions.push(crate::memory_search::project_or_global_clause(
            "project", idx,
        ));
        params_vec.push(Box::new(project.to_string()));
        idx += 1;
    }
    if let Some(memory_type) = memory_type {
        conditions.push(format!("memory_type = ?{idx}"));
        params_vec.push(Box::new(memory_type.to_string()));
        idx += 1;
    }
    if let Some(branch) = branch {
        conditions.push(format!("(branch = ?{idx} OR branch IS NULL)"));
        params_vec.push(Box::new(branch.to_string()));
        idx += 1;
    }

    let sql = format!(
        "SELECT id FROM memories
         WHERE {}
         ORDER BY updated_at_epoch DESC LIMIT ?{}",
        conditions.join(" AND "),
        idx
    );
    let mut stmt = conn.prepare(&sql)?;
    params_vec.push(Box::new(limit));
    let refs = crate::db::to_sql_refs(&params_vec);
    let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, i64>(0))?;
    for row in rows {
        ids.push(row?);
    }

    Ok(ids)
}
