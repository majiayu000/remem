use anyhow::Result;
use rusqlite::Connection;

use super::extract::extract_entities;

pub fn search_by_entity(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<i64>> {
    search_by_entity_filtered(conn, query, project, None, None, limit, false)
}

pub fn search_by_entity_filtered(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    limit: i64,
    include_inactive: bool,
) -> Result<Vec<i64>> {
    let query_entities = extract_entities(query, "");
    if query_entities.is_empty() {
        return search_by_query_words(
            conn,
            query,
            project,
            memory_type,
            branch,
            limit,
            include_inactive,
        );
    }

    let mut all_ids = Vec::new();
    for entity_name in &query_entities {
        let ids = query_memory_ids(
            conn,
            "e.canonical_name = ?1 COLLATE NOCASE".to_string(),
            Box::new(entity_name.clone()),
            project,
            memory_type,
            branch,
            limit,
            include_inactive,
        )?;
        for id in ids {
            if !all_ids.contains(&id) {
                all_ids.push(id);
            }
        }
    }
    Ok(all_ids)
}

fn search_by_query_words(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    limit: i64,
    include_inactive: bool,
) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    for word in query.split_whitespace() {
        if word.len() < 2 {
            continue;
        }
        let pattern = format!("%{}%", word);
        let matches = query_memory_ids(
            conn,
            "e.canonical_name LIKE ?1 COLLATE NOCASE".to_string(),
            Box::new(pattern),
            project,
            memory_type,
            branch,
            limit,
            include_inactive,
        )?;
        for id in matches {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    Ok(ids)
}

fn query_memory_ids(
    conn: &Connection,
    entity_condition: String,
    first_param: Box<dyn rusqlite::types::ToSql>,
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    limit: i64,
    include_inactive: bool,
) -> Result<Vec<i64>> {
    let mut conditions = vec![
        entity_condition,
        status_filter_sql(include_inactive).to_string(),
    ];
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![first_param];
    let mut idx = 2;
    if let Some(project) = project {
        conditions.push(project_filter_sql(idx));
        params_vec.push(Box::new(project.to_string()));
        idx += 1;
    }
    if let Some(memory_type) = memory_type {
        conditions.push(format!("m.memory_type = ?{idx}"));
        params_vec.push(Box::new(memory_type.to_string()));
        idx += 1;
    }
    if let Some(branch) = branch {
        conditions.push(branch_filter_sql(idx));
        params_vec.push(Box::new(branch.to_string()));
        idx += 1;
    }
    let sql = format!(
        "SELECT DISTINCT me.memory_id FROM memory_entities me
         JOIN entities e ON e.id = me.entity_id
         JOIN memories m ON m.id = me.memory_id
         WHERE {}
         LIMIT ?{}",
        conditions.join(" AND "),
        idx
    );
    params_vec.push(Box::new(limit));
    let refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|value| value.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, i64>(0))?;
    crate::db_query::collect_rows(rows)
}

fn project_filter_sql(param_idx: usize) -> String {
    format!("m.project = ?{idx}", idx = param_idx)
}

fn branch_filter_sql(param_idx: usize) -> String {
    format!("(m.branch = ?{idx} OR m.branch IS NULL)", idx = param_idx)
}

fn status_filter_sql(include_inactive: bool) -> &'static str {
    if include_inactive {
        "1=1"
    } else {
        "m.status = 'active'"
    }
}
