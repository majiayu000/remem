use anyhow::Result;
use rusqlite::Connection;

use super::super::common::WeightedRankedHit;
use super::SearchWeights;

pub(crate) fn usage_hits_for_retrieved_candidates(
    conn: &Connection,
    candidate_ids: &[i64],
    weights: SearchWeights,
) -> Result<Vec<WeightedRankedHit>> {
    if candidate_ids.is_empty() {
        return Ok(vec![]);
    }

    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let placeholders = candidate_ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            params.push(Box::new(*id) as Box<dyn rusqlite::types::ToSql>);
            format!("?{}", index + 1)
        })
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT id, COALESCE(access_count, 0), last_accessed_epoch
         FROM memories
         WHERE id IN ({placeholders})
           AND COALESCE(access_count, 0) > 0
           AND last_accessed_epoch IS NOT NULL"
    );
    let refs = crate::db::to_sql_refs(&params);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let now_epoch = chrono::Utc::now().timestamp();
    let half_life_days = weights.usage_recency_half_life_days.max(1.0);
    let mut scored = crate::db::query::collect_rows(rows)?
        .into_iter()
        .map(|(id, access_count, last_accessed_epoch)| {
            let age_days = ((now_epoch - last_accessed_epoch).max(0) as f64) / 86_400.0;
            let recency = 0.5_f64.powf(age_days / half_life_days);
            let score = (access_count.max(0) as f64).ln_1p() * recency;
            (id, score)
        })
        .filter(|(_, score)| *score > 0.0)
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    let max_score = scored.first().map(|(_, score)| *score).unwrap_or(0.0);
    if max_score <= 0.0 {
        return Ok(vec![]);
    }
    Ok(scored
        .into_iter()
        .map(|(id, score)| WeightedRankedHit {
            id,
            normalized_score: (score / max_score).clamp(0.0, 1.0),
        })
        .collect())
}
