use anyhow::Result;
use rusqlite::Connection;

use super::embedding::EmbeddingProfile;
use super::vector::VectorSearchFilters;

pub const VECTOR_SEARCH_CANDIDATE_LIMIT: usize = 4_096;
const VECTOR_SEARCH_MIN_CANDIDATES: usize = 512;
const VECTOR_SEARCH_BUCKETS: usize = 128;

pub(crate) fn vector_candidate_limit(limit: usize) -> usize {
    limit.clamp(VECTOR_SEARCH_MIN_CANDIDATES, VECTOR_SEARCH_CANDIDATE_LIMIT)
}

pub(crate) fn matching_memory_count(
    conn: &Connection,
    filters: VectorSearchFilters<'_>,
) -> Result<i64> {
    let (conditions, values) = memory_filter_conditions(filters, 1);
    let sql = format!(
        "SELECT COUNT(*) FROM memories m WHERE {}",
        conditions.join(" AND ")
    );
    let refs = crate::db::to_sql_refs(&values);
    Ok(conn.query_row(&sql, refs.as_slice(), |row| row.get(0))?)
}

pub(crate) fn select_candidate_ids(
    conn: &Connection,
    filters: VectorSearchFilters<'_>,
    profile: EmbeddingProfile<'_>,
    limit: usize,
) -> Result<Vec<i64>> {
    let limit = vector_candidate_limit(limit);
    let Some((min_id, max_id)) = embedding_id_bounds(conn, profile)? else {
        return Ok(Vec::new());
    };

    let buckets = limit.clamp(1, VECTOR_SEARCH_BUCKETS);
    let per_bucket = limit.div_ceil(buckets).max(1);
    let span = (max_id - min_id + 1).max(1);
    let mut ids = Vec::with_capacity(limit);

    for bucket in 0..buckets {
        if ids.len() >= limit {
            break;
        }
        let start = min_id + span.saturating_mul(bucket as i64) / buckets as i64;
        let mut end = min_id + span.saturating_mul((bucket + 1) as i64) / buckets as i64 - 1;
        if bucket + 1 == buckets {
            end = max_id;
        }
        append_bucket_ids(
            conn,
            filters,
            profile,
            start,
            end.max(start),
            per_bucket,
            limit,
            &mut ids,
        )?;
    }

    if ids.len() < limit {
        append_recent_ids(conn, filters, profile, limit, &mut ids)?;
    }

    ids.truncate(limit);
    Ok(ids)
}

pub(crate) fn matching_embedding_count(
    conn: &Connection,
    filters: VectorSearchFilters<'_>,
    profile: EmbeddingProfile<'_>,
) -> Result<i64> {
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(profile.model.to_string()),
        Box::new(profile.dimensions as i64),
    ];
    let (conditions, mut filter_values) = memory_filter_conditions(filters, 3);
    values.append(&mut filter_values);
    let sql = format!(
        "SELECT COUNT(*)
         FROM memory_embeddings e
         JOIN memories m ON m.id = e.memory_id
         WHERE e.model = ?1
           AND e.dimensions = ?2
           AND {}",
        conditions.join(" AND ")
    );
    let refs = crate::db::to_sql_refs(&values);
    Ok(conn.query_row(&sql, refs.as_slice(), |row| row.get(0))?)
}

fn embedding_id_bounds(
    conn: &Connection,
    profile: EmbeddingProfile<'_>,
) -> Result<Option<(i64, i64)>> {
    let (min_id, max_id): (Option<i64>, Option<i64>) = conn.query_row(
        "SELECT MIN(memory_id), MAX(memory_id)
         FROM memory_embeddings
         WHERE model = ?1
           AND dimensions = ?2",
        (&profile.model, profile.dimensions as i64),
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(min_id.zip(max_id))
}

fn append_bucket_ids(
    conn: &Connection,
    filters: VectorSearchFilters<'_>,
    profile: EmbeddingProfile<'_>,
    start: i64,
    end: i64,
    per_bucket: usize,
    total_limit: usize,
    ids: &mut Vec<i64>,
) -> Result<()> {
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(start),
        Box::new(end),
        Box::new(profile.model.to_string()),
        Box::new(profile.dimensions as i64),
    ];
    let (mut conditions, mut filter_values) = memory_filter_conditions(filters, 5);
    values.append(&mut filter_values);
    let limit_idx = values.len() + 1;
    values.push(Box::new(per_bucket as i64));
    conditions.insert(0, "e.memory_id BETWEEN ?1 AND ?2".to_string());
    conditions.insert(1, "e.model = ?3".to_string());
    conditions.insert(2, "e.dimensions = ?4".to_string());
    let sql = format!(
        "SELECT e.memory_id
         FROM memory_embeddings e
         JOIN memories m ON m.id = e.memory_id
         WHERE {}
         ORDER BY e.memory_id
         LIMIT ?{limit_idx}",
        conditions.join(" AND ")
    );
    append_ids_from_query(conn, &sql, &values, total_limit, ids)
}

fn append_recent_ids(
    conn: &Connection,
    filters: VectorSearchFilters<'_>,
    profile: EmbeddingProfile<'_>,
    total_limit: usize,
    ids: &mut Vec<i64>,
) -> Result<()> {
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(profile.model.to_string()),
        Box::new(profile.dimensions as i64),
    ];
    let (mut conditions, mut filter_values) = memory_filter_conditions(filters, 3);
    values.append(&mut filter_values);
    let limit_idx = values.len() + 1;
    values.push(Box::new(total_limit as i64));
    conditions.insert(0, "e.model = ?1".to_string());
    conditions.insert(1, "e.dimensions = ?2".to_string());
    let sql = format!(
        "SELECT e.memory_id
         FROM memory_embeddings e
         JOIN memories m ON m.id = e.memory_id
         WHERE {}
         ORDER BY e.memory_id DESC
         LIMIT ?{limit_idx}",
        conditions.join(" AND ")
    );
    append_ids_from_query(conn, &sql, &values, total_limit, ids)
}

fn append_ids_from_query(
    conn: &Connection,
    sql: &str,
    values: &[Box<dyn rusqlite::types::ToSql>],
    total_limit: usize,
    ids: &mut Vec<i64>,
) -> Result<()> {
    let refs = crate::db::to_sql_refs(values);
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, i64>(0))?;
    for row in rows {
        let id = row?;
        if !ids.contains(&id) {
            ids.push(id);
            if ids.len() >= total_limit {
                break;
            }
        }
    }
    Ok(())
}

fn memory_filter_conditions(
    filters: VectorSearchFilters<'_>,
    start_idx: usize,
) -> (Vec<String>, Vec<Box<dyn rusqlite::types::ToSql>>) {
    let mut conditions = vec![crate::memory::memory_current_filter_sql(
        "m.status",
        "m.expires_at_epoch",
        filters.include_stale,
    )];
    if !filters.include_stale {
        conditions.push(crate::memory::memory_state_key_current_filter_sql("m"));
    }
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = start_idx;
    if let Some(project) = filters.project {
        conditions.push(format!("(m.project = ?{idx} OR m.scope = 'global')"));
        values.push(Box::new(project.to_string()));
        idx += 1;
    }
    if let Some(branch) = filters.branch {
        conditions.push(format!("(m.branch = ?{idx} OR m.branch IS NULL)"));
        values.push(Box::new(branch.to_string()));
        idx += 1;
    }
    if let Some(memory_type) = filters.memory_type {
        conditions.push(format!("m.memory_type = ?{idx}"));
        values.push(Box::new(memory_type.to_string()));
    }
    (conditions, values)
}
