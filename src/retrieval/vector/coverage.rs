use anyhow::{bail, Result};
use rusqlite::{params, Connection};

use crate::retrieval::embedding::{
    embedding_provider_status, EmbeddingBackfillTarget, EmbeddingProviderStatus,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveEmbeddingCoverage {
    pub embedded: i64,
    pub total: i64,
    pub percent: f64,
    pub mixed_profile_count: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InactiveEmbeddingPruneReport {
    pub pruned: i64,
    pub active_model: String,
    pub active_dimensions: usize,
    pub coverage: ActiveEmbeddingCoverage,
}

pub fn active_embedding_coverage(conn: &Connection) -> Result<ActiveEmbeddingCoverage> {
    let status = embedding_provider_status()?;
    active_embedding_coverage_for_status(conn, &status)
}

pub fn active_embedding_coverage_for_status(
    conn: &Connection,
    status: &EmbeddingProviderStatus,
) -> Result<ActiveEmbeddingCoverage> {
    if !super::table_exists(conn, "memories")? {
        return Ok(ActiveEmbeddingCoverage {
            embedded: 0,
            total: 0,
            percent: 0.0,
            mixed_profile_count: 0,
        });
    }
    let total = searchable_memory_count(conn)?;
    if status.disabled || !super::table_exists(conn, "memory_embeddings")? {
        return Ok(ActiveEmbeddingCoverage {
            embedded: 0,
            total,
            percent: percent(0, total),
            mixed_profile_count: 0,
        });
    }
    let Some(model) = status.active_model_id.as_deref() else {
        return Ok(ActiveEmbeddingCoverage {
            embedded: 0,
            total,
            percent: percent(0, total),
            mixed_profile_count: embedding_profile_count(conn)?,
        });
    };
    let embedded = match status.active_dimensions {
        Some(dimensions) => conn.query_row(
            "SELECT COUNT(DISTINCT m.id)
             FROM memories m
             JOIN memory_embeddings e ON e.memory_id = m.id
             WHERE m.status IN ('active', 'stale', 'archived')
               AND e.model = ?1
               AND e.dimensions = ?2",
            params![model, dimensions as i64],
            |row| row.get(0),
        )?,
        None => conn.query_row(
            "SELECT COUNT(DISTINCT m.id)
             FROM memories m
             JOIN memory_embeddings e ON e.memory_id = m.id
             WHERE m.status IN ('active', 'stale', 'archived')
               AND e.model = ?1",
            [model],
            |row| row.get(0),
        )?,
    };
    Ok(ActiveEmbeddingCoverage {
        embedded,
        total,
        percent: percent(embedded, total),
        mixed_profile_count: embedding_profile_count(conn)?,
    })
}

fn searchable_memory_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE status IN ('active', 'stale', 'archived')",
        [],
        |row| row.get(0),
    )?)
}

fn embedding_profile_count(conn: &Connection) -> Result<i64> {
    if !super::table_exists(conn, "memory_embeddings")? {
        return Ok(0);
    }
    Ok(conn.query_row(
        "SELECT COUNT(*)
         FROM (
             SELECT model, dimensions
             FROM memory_embeddings
             GROUP BY model, dimensions
         )",
        [],
        |row| row.get(0),
    )?)
}

pub fn prune_inactive_memory_embeddings(
    conn: &Connection,
    target: &EmbeddingBackfillTarget,
) -> Result<InactiveEmbeddingPruneReport> {
    if !super::table_exists(conn, "memories")? || !super::table_exists(conn, "memory_embeddings")? {
        return Ok(InactiveEmbeddingPruneReport {
            pruned: 0,
            active_model: target.model.clone(),
            active_dimensions: target.dimensions,
            coverage: ActiveEmbeddingCoverage {
                embedded: 0,
                total: 0,
                percent: 0.0,
                mixed_profile_count: 0,
            },
        });
    }
    let coverage = embedding_coverage_for_target(conn, target)?;
    if coverage.embedded < coverage.total {
        bail!(
            "refusing to prune inactive embedding profiles before active coverage reaches 100%: {}/{} ({:.1}%)",
            coverage.embedded,
            coverage.total,
            coverage.percent
        );
    }
    let stale_or_missing = pending_reindex_count_for_target(conn, target)?;
    if stale_or_missing > 0 {
        bail!(
            "refusing to prune inactive embedding profiles while active profile has {stale_or_missing} missing or stale rows; run embedding backfill without --limit before pruning"
        );
    }
    let pruned = conn.execute(
        "DELETE FROM memory_embeddings
         WHERE rowid IN (
             SELECT e.rowid
             FROM memory_embeddings e
             JOIN memories m ON m.id = e.memory_id
             WHERE m.status IN ('active', 'stale', 'archived')
               AND NOT (e.model = ?1 AND e.dimensions = ?2)
         )",
        params![target.model.as_str(), target.dimensions as i64],
    )? as i64;
    Ok(InactiveEmbeddingPruneReport {
        pruned,
        active_model: target.model.clone(),
        active_dimensions: target.dimensions,
        coverage,
    })
}

fn embedding_coverage_for_target(
    conn: &Connection,
    target: &EmbeddingBackfillTarget,
) -> Result<ActiveEmbeddingCoverage> {
    let total = searchable_memory_count(conn)?;
    let embedded = conn.query_row(
        "SELECT COUNT(DISTINCT m.id)
         FROM memories m
         JOIN memory_embeddings e ON e.memory_id = m.id
         WHERE m.status IN ('active', 'stale', 'archived')
           AND e.model = ?1
           AND e.dimensions = ?2",
        params![target.model.as_str(), target.dimensions as i64],
        |row| row.get(0),
    )?;
    Ok(ActiveEmbeddingCoverage {
        embedded,
        total,
        percent: percent(embedded, total),
        mixed_profile_count: embedding_profile_count(conn)?,
    })
}

fn pending_reindex_count_for_target(
    conn: &Connection,
    target: &EmbeddingBackfillTarget,
) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*)
         FROM memories m
         LEFT JOIN memory_embeddings e
           ON e.memory_id = m.id
          AND e.model = ?1
          AND e.dimensions = ?2
         WHERE (e.memory_id IS NULL
                OR e.updated_at_epoch < m.updated_at_epoch)
           AND m.status IN ('active', 'stale', 'archived')",
        params![target.model.as_str(), target.dimensions as i64],
        |row| row.get(0),
    )?)
}

fn percent(numerator: i64, denominator: i64) -> f64 {
    if denominator <= 0 {
        0.0
    } else {
        (numerator as f64 * 100.0) / denominator as f64
    }
}
