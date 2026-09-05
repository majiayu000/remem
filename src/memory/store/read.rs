use std::collections::HashMap;

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
    idx =
        push_project_filter_required(conn, "project", project, idx, &mut conditions, &mut params)?;
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
    let (project_clause, next) = crate::project_alias::push_project_value_filter(
        conn,
        "project",
        project,
        idx,
        &mut params,
    )?;
    conditions.push(project_clause);
    idx = next;
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
    let (project_clause, next) = crate::project_alias::push_project_value_filter(
        conn,
        "project",
        project,
        idx,
        &mut params,
    )?;
    conditions.push(project_clause);
    idx = next;
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
    idx =
        push_project_filter_required(conn, "project", project, idx, &mut conditions, &mut params)?;

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

pub(crate) fn memory_details_with_topic_traces(
    conn: &Connection,
    memories: &[Memory],
    requested_project: Option<&str>,
) -> Result<serde_json::Value> {
    const TRACE_LIMIT: i64 = 12;
    let mut value = serde_json::to_value(memories)?;
    let Some(items) = value.as_array_mut() else {
        return Ok(value);
    };
    let memory_ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
    let temporal_facts = current_temporal_facts_by_memory_id(conn, &memory_ids, requested_project)?;
    let mut trace_cache = HashMap::new();
    let as_of_epoch = chrono::Utc::now().timestamp();
    for (item, memory) in items.iter_mut().zip(memories) {
        let visibility = crate::truth::classify_memory(conn, memory.id, as_of_epoch)?;
        item["classification"] = serde_json::json!(visibility.classification);
        item["classification_reason"] =
            serde_json::Value::String(visibility.reason.as_str().to_string());
        item["current_context_eligible"] =
            serde_json::Value::Bool(visibility.current_context_eligible);
        if let Some(facts) = temporal_facts.get(&memory.id) {
            if !facts.is_empty() {
                item["temporal_facts"] = serde_json::to_value(facts)?;
            }
        }
        let Some(topic_key) = memory.topic_key.as_deref() else {
            continue;
        };
        let trace_project = match requested_project {
            Some(project) if project == memory.project => project,
            Some(_) => continue,
            None => memory.project.as_str(),
        };
        let cache_key = (trace_project.to_string(), topic_key.to_string());
        if !trace_cache.contains_key(&cache_key) {
            let trace =
                crate::db::load_trace_by_topic_key(conn, trace_project, topic_key, TRACE_LIMIT)?;
            trace_cache.insert(cache_key.clone(), trace);
        }
        let trace = trace_cache
            .get(&cache_key)
            .expect("trace cache should contain loaded key");
        if !trace.is_empty() {
            item["topic_trace"] = serde_json::to_value(trace)?;
        }
    }
    Ok(value)
}

#[derive(serde::Serialize)]
struct MemoryTemporalFactDetail {
    project: String,
    subject: String,
    predicate: String,
    object: String,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
    learned_at_epoch: i64,
    confidence: f64,
    status: String,
}

fn current_temporal_facts_by_memory_id(
    conn: &Connection,
    memory_ids: &[i64],
    requested_project: Option<&str>,
) -> Result<HashMap<i64, Vec<MemoryTemporalFactDetail>>> {
    if memory_ids.is_empty()
        || !crate::retrieval::temporal::sqlite_table_exists(conn, "memory_facts")?
    {
        return Ok(HashMap::new());
    }
    let placeholders = (1..=memory_ids.len())
        .map(|idx| format!("?{idx}"))
        .collect::<Vec<_>>()
        .join(", ");
    let has_invalidated_at_epoch = crate::memory::facts::invalidated_at_epoch_available(conn)?;
    let mut conditions = vec![
        format!("source_memory_id IN ({placeholders})"),
        crate::memory::facts::current_fact_filter_sql("", has_invalidated_at_epoch),
    ];
    let mut params = memory_ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect::<Vec<_>>();
    let now_idx = memory_ids.len() + 1;
    conditions.push(format!(
        "(valid_from_epoch IS NULL OR valid_from_epoch <= ?{now_idx})"
    ));
    conditions.push(format!(
        "(valid_to_epoch IS NULL OR valid_to_epoch > ?{now_idx})"
    ));
    params.push(Box::new(chrono::Utc::now().timestamp()));
    let mut idx = now_idx + 1;
    if let Some(project) = requested_project {
        conditions.push(format!("project = ?{idx}"));
        params.push(Box::new(project.to_string()));
        idx += 1;
    }
    let sql = format!(
        "SELECT source_memory_id, project, subject, predicate, object, valid_from_epoch,
                valid_to_epoch, learned_at_epoch, confidence, status
         FROM memory_facts
         WHERE {}
         ORDER BY source_memory_id, COALESCE(valid_from_epoch, learned_at_epoch) DESC,
                  confidence DESC, id DESC
         LIMIT ?{idx}",
        conditions.join(" AND ")
    );
    params.push(Box::new(
        (memory_ids.len() as i64).saturating_mul(12).max(12),
    ));
    let refs = crate::db::to_sql_refs(&params);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            MemoryTemporalFactDetail {
                project: row.get(1)?,
                subject: row.get(2)?,
                predicate: row.get(3)?,
                object: row.get(4)?,
                valid_from_epoch: row.get(5)?,
                valid_to_epoch: row.get(6)?,
                learned_at_epoch: row.get(7)?,
                confidence: row.get(8)?,
                status: row.get(9)?,
            },
        ))
    })?;
    let mut facts = HashMap::new();
    for row in rows {
        let (memory_id, fact) = row?;
        facts.entry(memory_id).or_insert_with(Vec::new).push(fact);
    }
    Ok(facts)
}

#[cfg(test)]
mod tests;
