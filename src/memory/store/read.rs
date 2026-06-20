use anyhow::Result;
use rusqlite::Connection;

use crate::memory::types::{map_memory_row, Memory, MEMORY_COLS};
use crate::retrieval::memory_search::push_project_filter_required;

pub fn get_recent_memories(conn: &Connection, project: &str, limit: i64) -> Result<Vec<Memory>> {
    list_memories(conn, project, None, limit, 0, false, None)
}

pub fn mark_memories_accessed(conn: &Connection, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }

    let now = chrono::Utc::now().timestamp();
    let placeholders = (2..ids.len() + 2)
        .map(|idx| format!("?{idx}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "UPDATE memories
         SET last_accessed_epoch = ?1,
             access_count = COALESCE(access_count, 0) + 1
         WHERE id IN ({placeholders})"
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now)];
    params.extend(
        ids.iter()
            .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>),
    );
    let refs = crate::db::to_sql_refs(&params);
    Ok(conn.execute(&sql, refs.as_slice())?)
}

pub fn get_recent_memories_excluding_types(
    conn: &Connection,
    project: &str,
    excluded_types: &[&str],
    limit: i64,
) -> Result<Vec<Memory>> {
    let mut conditions = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;
    idx = push_project_filter_required("project", project, idx, &mut conditions, &mut params);
    conditions.push(crate::memory::memory_current_filter_sql(
        "status",
        "expires_at_epoch",
        false,
    ));
    conditions.push(crate::memory::suppression::memory_policy_filter_sql(
        "memories",
    ));

    if !excluded_types.is_empty() {
        let placeholders: Vec<String> = excluded_types
            .iter()
            .map(|memory_type| {
                let placeholder = format!("?{idx}");
                params.push(Box::new((*memory_type).to_string()));
                idx += 1;
                placeholder
            })
            .collect();
        conditions.push(format!("memory_type NOT IN ({})", placeholders.join(", ")));
    }

    params.push(Box::new(limit));
    let sql = format!(
        "SELECT {} FROM memories \
         WHERE {} \
         ORDER BY updated_at_epoch DESC LIMIT ?{}",
        MEMORY_COLS,
        conditions.join(" AND "),
        idx,
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), map_memory_row)?;
    crate::db::query::collect_rows(rows)
}

pub fn get_recent_project_memories_excluding_types(
    conn: &Connection,
    project: &str,
    excluded_types: &[&str],
    limit: i64,
) -> Result<Vec<Memory>> {
    let mut conditions = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;
    conditions.push(format!("project = ?{idx}"));
    params.push(Box::new(project.to_string()));
    idx += 1;
    conditions.push("COALESCE(scope, 'project') != 'global'".to_string());
    conditions.push(crate::memory::memory_current_filter_sql(
        "status",
        "expires_at_epoch",
        false,
    ));
    conditions.push(crate::memory::suppression::memory_policy_filter_sql(
        "memories",
    ));

    if !excluded_types.is_empty() {
        let placeholders: Vec<String> = excluded_types
            .iter()
            .map(|memory_type| {
                let placeholder = format!("?{idx}");
                params.push(Box::new((*memory_type).to_string()));
                idx += 1;
                placeholder
            })
            .collect();
        conditions.push(format!("memory_type NOT IN ({})", placeholders.join(", ")));
    }

    params.push(Box::new(limit));
    let sql = format!(
        "SELECT {} FROM memories \
         WHERE {} \
         ORDER BY updated_at_epoch DESC LIMIT ?{}",
        MEMORY_COLS,
        conditions.join(" AND "),
        idx,
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), map_memory_row)?;
    crate::db::query::collect_rows(rows)
}

pub fn search_project_memories_excluding_types(
    conn: &Connection,
    project: &str,
    query: &str,
    excluded_types: &[&str],
    limit: i64,
) -> Result<Vec<Memory>> {
    if query.trim().is_empty() {
        return Ok(vec![]);
    }

    let mut conditions = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;
    conditions.push(format!("project = ?{idx}"));
    params.push(Box::new(project.to_string()));
    idx += 1;
    conditions.push("COALESCE(scope, 'project') != 'global'".to_string());
    conditions.push(crate::memory::memory_current_filter_sql(
        "status",
        "expires_at_epoch",
        false,
    ));
    conditions.push(crate::memory::suppression::memory_policy_filter_sql(
        "memories",
    ));

    let like_pattern = format!("%{query}%");
    conditions.push(format!("(title LIKE ?{idx} OR content LIKE ?{idx})"));
    params.push(Box::new(like_pattern));
    idx += 1;

    if !excluded_types.is_empty() {
        let placeholders: Vec<String> = excluded_types
            .iter()
            .map(|memory_type| {
                let placeholder = format!("?{idx}");
                params.push(Box::new((*memory_type).to_string()));
                idx += 1;
                placeholder
            })
            .collect();
        conditions.push(format!("memory_type NOT IN ({})", placeholders.join(", ")));
    }

    params.push(Box::new(limit));
    let sql = format!(
        "SELECT {} FROM memories \
         WHERE {} \
         ORDER BY updated_at_epoch DESC LIMIT ?{}",
        MEMORY_COLS,
        conditions.join(" AND "),
        idx,
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), map_memory_row)?;
    crate::db::query::collect_rows(rows)
}

pub fn get_memories_by_type(
    conn: &Connection,
    project: &str,
    memory_type: &str,
    limit: i64,
) -> Result<Vec<Memory>> {
    list_memories(conn, project, Some(memory_type), limit, 0, false, None)
}

pub fn list_memories(
    conn: &Connection,
    project: &str,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_inactive: bool,
    branch: Option<&str>,
) -> Result<Vec<Memory>> {
    list_memories_with_suppressed_policy(
        conn,
        project,
        memory_type,
        limit,
        offset,
        include_inactive,
        branch,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn list_memories_with_suppressed_policy(
    conn: &Connection,
    project: &str,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_inactive: bool,
    branch: Option<&str>,
    include_suppressed: bool,
) -> Result<Vec<Memory>> {
    let mut conditions = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;
    idx = push_project_filter_required("project", project, idx, &mut conditions, &mut params);

    conditions.push(crate::memory::memory_current_filter_sql(
        "status",
        "expires_at_epoch",
        include_inactive,
    ));
    if !include_suppressed {
        conditions.push(crate::memory::suppression::memory_policy_filter_sql(
            "memories",
        ));
    }
    if let Some(memory_type) = memory_type {
        conditions.push(format!("memory_type = ?{idx}"));
        params.push(Box::new(memory_type.to_string()));
        idx += 1;
    }
    if let Some(branch) = branch {
        conditions.push(format!("(branch = ?{idx} OR branch IS NULL)"));
        params.push(Box::new(branch.to_string()));
        idx += 1;
    }

    params.push(Box::new(limit));
    params.push(Box::new(offset.max(0)));
    let sql = format!(
        "SELECT {} FROM memories \
         WHERE {} \
         ORDER BY updated_at_epoch DESC LIMIT ?{} OFFSET ?{}",
        MEMORY_COLS,
        conditions.join(" AND "),
        idx,
        idx + 1,
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), map_memory_row)?;
    crate::db::query::collect_rows(rows)
}

pub fn get_memories_by_ids(
    conn: &Connection,
    ids: &[i64],
    project: Option<&str>,
) -> Result<Vec<Memory>> {
    get_memories_by_ids_with_suppressed_policy(conn, ids, project, false)
}

pub fn get_memories_by_ids_with_suppressed_policy(
    conn: &Connection,
    ids: &[i64],
    project: Option<&str>,
    include_suppressed: bool,
) -> Result<Vec<Memory>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }

    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let mut conditions = vec![format!("id IN ({})", placeholders.join(", "))];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();

    if let Some(project) = project {
        let idx = ids.len() + 1;
        conditions.push(format!("(project = ?{idx} OR scope = 'global')"));
        params.push(Box::new(project.to_string()));
    }
    if !include_suppressed {
        conditions.push(crate::memory::suppression::memory_policy_filter_sql(
            "memories",
        ));
    }

    let sql = format!(
        "SELECT {} FROM memories WHERE {} ORDER BY updated_at_epoch DESC",
        MEMORY_COLS,
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), map_memory_row)?;
    crate::db::query::collect_rows(rows)
}

#[cfg(test)]
mod tests;
