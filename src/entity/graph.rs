use anyhow::Result;
use rusqlite::Connection;

pub fn expand_via_entity_graph(
    conn: &Connection,
    seed_memory_ids: &[i64],
    exclude_ids: &[i64],
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<i64>> {
    expand_via_entity_graph_filtered(
        conn,
        seed_memory_ids,
        exclude_ids,
        project,
        None,
        None,
        limit,
        false,
    )
}

pub fn expand_via_entity_graph_filtered(
    conn: &Connection,
    seed_memory_ids: &[i64],
    exclude_ids: &[i64],
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    limit: i64,
    include_inactive: bool,
) -> Result<Vec<i64>> {
    if seed_memory_ids.is_empty() {
        return Ok(vec![]);
    }

    let entity_ids = load_seed_entity_ids(conn, seed_memory_ids)?;
    if entity_ids.is_empty() {
        return Ok(vec![]);
    }

    let exclude_set: std::collections::HashSet<i64> = exclude_ids.iter().copied().collect();
    let seed_set: std::collections::HashSet<i64> = seed_memory_ids.iter().copied().collect();
    let sql = build_expand_sql(
        entity_ids.len(),
        project,
        memory_type,
        branch,
        include_inactive,
    );
    let params_vec = build_expand_params(&entity_ids, project, memory_type, branch, limit * 3);
    let refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|value| value.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, i64>(0))?;
    let mut expanded_ids = Vec::new();
    for row in rows {
        let id = row?;
        if !seed_set.contains(&id) && !exclude_set.contains(&id) {
            expanded_ids.push(id);
            if expanded_ids.len() >= limit as usize {
                break;
            }
        }
    }
    Ok(expanded_ids)
}

fn load_seed_entity_ids(conn: &Connection, seed_memory_ids: &[i64]) -> Result<Vec<i64>> {
    let placeholders: Vec<String> = (1..=seed_memory_ids.len())
        .map(|i| format!("?{i}"))
        .collect();
    let sql = format!(
        "SELECT DISTINCT entity_id FROM memory_entities WHERE memory_id IN ({})",
        placeholders.join(", ")
    );
    let params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = seed_memory_ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    let refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|value| value.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, i64>(0))?;
    crate::db_query::collect_rows(rows)
}

fn build_expand_sql(
    entity_count: usize,
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    include_inactive: bool,
) -> String {
    let entity_placeholders: Vec<String> = (1..=entity_count).map(|i| format!("?{i}")).collect();
    let mut conditions = vec![
        format!("me.entity_id IN ({})", entity_placeholders.join(", ")),
        status_filter_sql(include_inactive).to_string(),
    ];
    let mut idx = entity_count + 1;
    if project.is_some() {
        conditions.push(project_filter_sql(idx));
        idx += 1;
    }
    if memory_type.is_some() {
        conditions.push(format!("m.memory_type = ?{idx}"));
        idx += 1;
    }
    if branch.is_some() {
        conditions.push(branch_filter_sql(idx));
        idx += 1;
    }
    format!(
        "SELECT me.memory_id, COUNT(DISTINCT me.entity_id) as shared_count
         FROM memory_entities me
         JOIN memories m ON m.id = me.memory_id
         WHERE {}
         GROUP BY me.memory_id
         ORDER BY shared_count DESC
         LIMIT ?{}",
        conditions.join(" AND "),
        idx
    )
}

fn build_expand_params(
    entity_ids: &[i64],
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    limit: i64,
) -> Vec<Box<dyn rusqlite::types::ToSql>> {
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = entity_ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    if let Some(project) = project {
        params_vec.push(Box::new(project.to_string()));
    }
    if let Some(memory_type) = memory_type {
        params_vec.push(Box::new(memory_type.to_string()));
    }
    if let Some(branch) = branch {
        params_vec.push(Box::new(branch.to_string()));
    }
    params_vec.push(Box::new(limit));
    params_vec
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
