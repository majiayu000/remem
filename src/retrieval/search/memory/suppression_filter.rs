use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{FtsMemoryHit, Memory};
use crate::retrieval::search::common::WeightedRankedHit;

pub(super) fn ids(conn: &Connection, ids: Vec<i64>, include_suppressed: bool) -> Result<Vec<i64>> {
    if include_suppressed || ids.is_empty() {
        return Ok(ids);
    }
    let suppressed = crate::memory::suppression::active_suppressed_memory_ids(conn, &ids)?;
    Ok(ids
        .into_iter()
        .filter(|id| !suppressed.contains(id))
        .collect())
}

pub(super) fn weighted_hits(
    conn: &Connection,
    hits: Vec<WeightedRankedHit>,
    include_suppressed: bool,
) -> Result<Vec<WeightedRankedHit>> {
    if include_suppressed || hits.is_empty() {
        return Ok(hits);
    }
    let memory_ids = hits.iter().map(|hit| hit.id).collect::<Vec<_>>();
    let suppressed = crate::memory::suppression::active_suppressed_memory_ids(conn, &memory_ids)?;
    Ok(hits
        .into_iter()
        .filter(|hit| !suppressed.contains(&hit.id))
        .collect())
}

pub(super) fn memories(
    conn: &Connection,
    memories: Vec<Memory>,
    include_suppressed: bool,
) -> Result<Vec<Memory>> {
    if include_suppressed || memories.is_empty() {
        return Ok(memories);
    }
    let ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
    let suppressed = crate::memory::suppression::active_suppressed_memory_ids(conn, &ids)?;
    Ok(memories
        .into_iter()
        .filter(|memory| !suppressed.contains(&memory.id))
        .collect())
}

pub(super) fn fts_hits(
    conn: &Connection,
    hits: Vec<FtsMemoryHit>,
    include_suppressed: bool,
) -> Result<Vec<FtsMemoryHit>> {
    if include_suppressed || hits.is_empty() {
        return Ok(hits);
    }
    let ids = hits.iter().map(|hit| hit.memory.id).collect::<Vec<_>>();
    let suppressed = crate::memory::suppression::active_suppressed_memory_ids(conn, &ids)?;
    Ok(hits
        .into_iter()
        .filter(|hit| !suppressed.contains(&hit.memory.id))
        .collect())
}

pub(super) fn ordered(
    conn: &Connection,
    ordered: Vec<Memory>,
    fused: &[(i64, f64)],
    include_suppressed: bool,
) -> Result<(Vec<Memory>, Vec<(i64, f64)>)> {
    if include_suppressed || ordered.is_empty() {
        return Ok((ordered, fused.to_vec()));
    }
    let ids = ordered.iter().map(|memory| memory.id).collect::<Vec<_>>();
    let suppressed = crate::memory::suppression::active_suppressed_memory_ids(conn, &ids)?;
    if suppressed.is_empty() {
        return Ok((ordered, fused.to_vec()));
    }
    let ordered = ordered
        .into_iter()
        .filter(|memory| !suppressed.contains(&memory.id))
        .collect::<Vec<_>>();
    let fused = fused
        .iter()
        .copied()
        .filter(|(id, _)| !suppressed.contains(id))
        .collect::<Vec<_>>();
    Ok((ordered, fused))
}
