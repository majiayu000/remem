use anyhow::Result;
use rusqlite::Connection;

use super::query::ContextMemoryRow;

const MAX_ABSTENTION_RESCUE_VECTOR_DISTANCE: f32 = 0.72;

pub(super) fn filter_recent_rows_by_task_embedding(
    conn: &Connection,
    task_query: &str,
    recent: Vec<ContextMemoryRow>,
    limit: usize,
) -> Result<Vec<ContextMemoryRow>> {
    let candidate_ids = recent.iter().map(|row| row.memory.id).collect::<Vec<_>>();
    let matched_ids = rank_stored_embedding_matches(conn, task_query, &candidate_ids, limit)?;
    let ranks = matched_ids
        .into_iter()
        .enumerate()
        .map(|(rank, id)| (id, rank))
        .collect::<std::collections::HashMap<_, _>>();
    let mut matched_rows = recent
        .into_iter()
        .filter(|row| ranks.contains_key(&row.memory.id))
        .collect::<Vec<_>>();
    matched_rows.sort_by_key(|row| ranks.get(&row.memory.id).copied().unwrap_or(usize::MAX));
    Ok(matched_rows)
}

pub(super) fn rank_stored_embedding_matches(
    conn: &Connection,
    query: &str,
    candidate_ids: &[i64],
    limit: usize,
) -> Result<Vec<i64>> {
    if limit == 0 || query.trim().is_empty() || candidate_ids.is_empty() {
        return Ok(Vec::new());
    }
    if crate::retrieval::embedding::embedding_provider_status()?.disabled {
        return Ok(Vec::new());
    }

    let query_embedding = crate::retrieval::embedding::embed_query(query)?;
    let profile = query_embedding.profile();
    let mut ids = candidate_ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let placeholders = (0..ids.len())
        .map(|idx| format!("?{}", idx + 3))
        .collect::<Vec<_>>()
        .join(", ");
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(profile.model.to_string()),
        Box::new(profile.dimensions as i64),
    ];
    params.extend(
        ids.iter()
            .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>),
    );
    let sql = format!(
        "SELECT memory_id, embedding, dimensions
         FROM memory_embeddings
         WHERE model = ?1
           AND dimensions = ?2
           AND memory_id IN ({placeholders})"
    );
    let refs = crate::db::to_sql_refs(&params);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;

    let mut matches = Vec::new();
    for row in crate::db::query::collect_rows(rows)? {
        let (memory_id, blob, dimensions) = row;
        let embedding = crate::retrieval::vector::decode_embedding(&blob, dimensions)?;
        let distance =
            crate::retrieval::vector::cosine_distance(query_embedding.values(), &embedding)?;
        if distance <= MAX_ABSTENTION_RESCUE_VECTOR_DISTANCE {
            matches.push((memory_id, distance));
        }
    }
    matches.sort_by(|left, right| {
        left.1
            .partial_cmp(&right.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    Ok(matches
        .into_iter()
        .take(limit)
        .map(|(memory_id, _)| memory_id)
        .collect())
}
