use anyhow::{bail, Result};
use rusqlite::{params, Connection};

use crate::retrieval::embedding::{
    configured_backfill_target_with_fallback_cache, embedding_provider_status,
    embedding_provider_status_without_probe, resolve_embedding_config, EmbeddingBackfillTarget,
    EmbeddingConfig, EmbeddingFallbackCache, EmbeddingProviderStatus,
};
#[cfg(feature = "local-onnx")]
use crate::retrieval::embedding::{with_configured_model_read_lock, EmbeddingProvider};

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
    let config = resolve_embedding_config()?;
    #[cfg(feature = "local-onnx")]
    let pin_local_model_state = match config.provider {
        EmbeddingProvider::Local => true,
        EmbeddingProvider::Auto => {
            embedding_provider_status_without_probe()?.active_provider
                != EmbeddingProvider::OpenAi.label()
        }
        EmbeddingProvider::FeatureHash | EmbeddingProvider::OpenAi | EmbeddingProvider::Off => {
            false
        }
    };
    #[cfg(feature = "local-onnx")]
    if pin_local_model_state {
        return with_configured_model_read_lock(&config, || {
            prune_inactive_memory_embeddings_pinned(conn, target, &config)
        });
    }
    prune_inactive_memory_embeddings_pinned(conn, target, &config)
}

fn prune_inactive_memory_embeddings_pinned(
    conn: &Connection,
    target: &EmbeddingBackfillTarget,
    pinned_config: &EmbeddingConfig,
) -> Result<InactiveEmbeddingPruneReport> {
    ensure_current_prune_target(target, pinned_config)?;
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
    let coverage = active_embedding_coverage_for_target(conn, target)?;
    if coverage.embedded < coverage.total {
        bail!(
            "refusing to prune inactive embedding profiles before active coverage reaches 100%: {}/{} ({:.1}%)",
            coverage.embedded,
            coverage.total,
            coverage.percent
        );
    }
    let stale_or_missing = super::pending_memory_embedding_reindex_count_for_target(conn, target)?;
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

pub fn active_embedding_coverage_for_target(
    conn: &Connection,
    target: &EmbeddingBackfillTarget,
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
    if target.dimensions == 0 || !super::table_exists(conn, "memory_embeddings")? {
        return Ok(ActiveEmbeddingCoverage {
            embedded: 0,
            total,
            percent: percent(0, total),
            mixed_profile_count: embedding_profile_count(conn)?,
        });
    }
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

fn ensure_current_prune_target(
    target: &EmbeddingBackfillTarget,
    pinned_config: &EmbeddingConfig,
) -> Result<()> {
    let config_before = resolve_embedding_config()?;
    if &config_before != pinned_config {
        bail!(
            "refusing to prune embedding profiles because the embedding configuration changed before the model-state pin was acquired"
        );
    }
    let status_before = embedding_provider_status_without_probe()?;
    if status_before.disabled {
        bail!("cannot prune embedding profiles while embedding provider is off");
    }
    if status_before.degraded {
        bail!(
            "refusing to prune embedding profiles while the current provider is degraded: {}",
            status_before
                .degradation_reason
                .as_deref()
                .or(status_before.unavailable_reason.as_deref())
                .unwrap_or("unknown provider degradation")
        );
    }

    let mut fallback_cache = EmbeddingFallbackCache::default();
    let current = configured_backfill_target_with_fallback_cache(&mut fallback_cache)?;
    let config_after = resolve_embedding_config()?;
    let status_after = embedding_provider_status_without_probe()?;
    if config_after != config_before || status_after != status_before {
        bail!(
            "refusing to prune embedding profiles because the embedding configuration or active profile changed while resolving the current target"
        );
    }
    if let Some(fallback_target) = fallback_cache.call_failure_fallback_target() {
        bail!(
            "refusing to prune embedding profiles after typed provider fallback selected model={} dimensions={}",
            fallback_target.model,
            fallback_target.dimensions
        );
    }
    if &current != target {
        bail!(
            "refusing to prune stale target model={} dimensions={}; current embedding profile is model={} dimensions={}",
            target.model,
            target.dimensions,
            current.model,
            current.dimensions
        );
    }
    Ok(())
}

fn percent(numerator: i64, denominator: i64) -> f64 {
    if denominator <= 0 {
        0.0
    } else {
        (numerator as f64 * 100.0) / denominator as f64
    }
}
