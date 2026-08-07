use std::time::Instant;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::retrieval::embedding::{
    EmbeddingBackfillTarget, EmbeddingConfig, EmbeddingFallbackCache, EmbeddingProviderStatus,
};

use super::reindex::{
    prepare_memory_embedding_batch, select_memory_embedding_reindex_candidates,
    PreparedMemoryEmbedding,
};

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingReindexReport {
    pub selected: usize,
    pub processed: usize,
    pub model: String,
    pub dimensions: usize,
    pub timings: Vec<crate::perf::PhaseTiming>,
}

#[derive(Debug)]
pub(crate) struct EmbeddingBackfillSession {
    target: EmbeddingBackfillTarget,
    fallback_cache: EmbeddingFallbackCache,
    pinned_config: EmbeddingConfig,
    pinned_status: EmbeddingProviderStatus,
    prune_block_reason: Option<String>,
    disabled: bool,
}

impl EmbeddingBackfillSession {
    pub(crate) fn start() -> Result<Self> {
        let config_before = crate::retrieval::embedding::resolve_embedding_config()?;
        let status_before = crate::retrieval::embedding::embedding_provider_status_without_probe()?;
        if status_before.disabled {
            if let Some(error) =
                crate::retrieval::embedding::disabled_provider_status_error(&status_before)
            {
                if !crate::retrieval::embedding::is_embedding_provider_off_error(&error) {
                    return Err(error);
                }
            }
            return Ok(Self {
                target: EmbeddingBackfillTarget {
                    model: "off".to_string(),
                    dimensions: 0,
                },
                fallback_cache: EmbeddingFallbackCache::default(),
                pinned_config: config_before,
                prune_block_reason: status_before
                    .degradation_reason
                    .clone()
                    .or_else(|| Some("embedding provider is off".to_string())),
                pinned_status: status_before,
                disabled: true,
            });
        }

        let mut fallback_cache = EmbeddingFallbackCache::default();
        let target = crate::retrieval::embedding::configured_backfill_target_with_fallback_cache(
            &mut fallback_cache,
        )?;
        let config_after = crate::retrieval::embedding::resolve_embedding_config()?;
        let status_after = crate::retrieval::embedding::embedding_provider_status_without_probe()?;
        if config_after != config_before || status_after != status_before {
            anyhow::bail!(
                "embedding configuration or active profile changed while pinning backfill target; refusing to start"
            );
        }

        let fallback_target = fallback_cache.call_failure_fallback_target();
        if let Some(fallback_target) = fallback_target.as_ref() {
            if fallback_target != &target {
                anyhow::bail!(
                    "pinned embedding profile changed while starting backfill: expected model={} dimensions={}, got fallback model={} dimensions={}",
                    target.model,
                    target.dimensions,
                    fallback_target.model,
                    fallback_target.dimensions
                );
            }
        }
        let prune_block_reason = status_after
            .degraded
            .then(|| {
                status_after
                    .degradation_reason
                    .clone()
                    .unwrap_or_else(|| "embedding provider is degraded".to_string())
            })
            .or_else(|| {
                fallback_target.map(|fallback_target| {
                    format!(
                        "typed provider fallback selected model={} dimensions={}",
                        fallback_target.model, fallback_target.dimensions
                    )
                })
            });
        Ok(Self {
            target,
            fallback_cache,
            pinned_config: config_after,
            pinned_status: status_after,
            prune_block_reason,
            disabled: false,
        })
    }

    pub(crate) fn target(&self) -> &EmbeddingBackfillTarget {
        &self.target
    }

    fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub(crate) fn ensure_environment_unchanged(&mut self, phase: &str) -> Result<()> {
        let current_config = crate::retrieval::embedding::resolve_embedding_config()?;
        if current_config != self.pinned_config {
            let reason = format!(
                "embedding configuration changed {phase} for pinned model={} dimensions={}",
                self.target.model, self.target.dimensions
            );
            self.prune_block_reason.get_or_insert(reason.clone());
            anyhow::bail!("{reason}");
        }
        let current_status =
            crate::retrieval::embedding::embedding_provider_status_without_probe()?;
        if current_status != self.pinned_status {
            let current_dimensions = current_status
                .active_dimensions
                .map(|dimensions| dimensions.to_string())
                .unwrap_or_else(|| "<none>".to_string());
            let reason = format!(
                "active embedding provider/profile changed {phase}: pinned model={} dimensions={}, current model={} dimensions={}",
                self.target.model,
                self.target.dimensions,
                current_status.active_model_id.as_deref().unwrap_or("<none>"),
                current_dimensions
            );
            self.prune_block_reason.get_or_insert(reason.clone());
            anyhow::bail!("{reason}");
        }
        Ok(())
    }

    fn observe_call_failure_fallback(&mut self) -> Result<()> {
        let Some(fallback_target) = self.fallback_cache.call_failure_fallback_target() else {
            return Ok(());
        };
        self.prune_block_reason.get_or_insert_with(|| {
            format!(
                "typed provider fallback selected model={} dimensions={}",
                fallback_target.model, fallback_target.dimensions
            )
        });
        if fallback_target != self.target {
            anyhow::bail!(
                "pinned embedding profile changed after typed fallback: expected model={} dimensions={}, got model={} dimensions={}",
                self.target.model,
                self.target.dimensions,
                fallback_target.model,
                fallback_target.dimensions
            );
        }
        Ok(())
    }

    pub(crate) fn ensure_prune_preconditions(&mut self) -> Result<()> {
        self.ensure_environment_unchanged("before pruning")?;
        if self.disabled {
            anyhow::bail!("cannot prune embedding profiles while embedding provider is off");
        }
        if let Some(reason) = self.prune_block_reason.as_deref() {
            anyhow::bail!(
                "refusing to prune embedding profiles after an unsafe provider transition: {reason}"
            );
        }
        Ok(())
    }
}

pub fn backfill_missing_memory_embeddings(conn: &Connection, limit: i64) -> Result<usize> {
    reindex_memory_embeddings(conn, limit)
}

pub fn reindex_memory_embeddings(conn: &Connection, limit: i64) -> Result<usize> {
    let mut remaining_limit = limit.max(0);
    if remaining_limit == 0
        || !super::table_exists(conn, "memories")?
        || !super::table_exists(conn, "memory_embeddings")?
    {
        return Ok(0);
    }
    let mut session = EmbeddingBackfillSession::start()?;
    let mut processed = 0usize;
    while remaining_limit > 0 {
        let batch_limit = remaining_limit.min(super::EMBEDDING_REINDEX_WRITE_BATCH_SIZE as i64);
        let report =
            reindex_memory_embeddings_with_session_report(conn, batch_limit, &mut session)?;
        if report.processed == 0 {
            break;
        }
        processed += report.processed;
        remaining_limit -= report.processed as i64;
        if report.processed < batch_limit as usize {
            break;
        }
    }
    session.ensure_environment_unchanged("before finalizing backfill")?;
    Ok(processed)
}

pub fn reindex_memory_embeddings_with_report(
    conn: &Connection,
    limit: i64,
) -> Result<EmbeddingReindexReport> {
    let total_start = Instant::now();
    let mut timings = vec![];
    if crate::retrieval::embedding::provider_disabled_or_error()? {
        crate::perf::push_elapsed(&mut timings, "total", total_start);
        return Ok(empty_report("off", 0, timings));
    }
    if !super::table_exists(conn, "memories")?
        || !super::table_exists(conn, "memory_embeddings")?
        || limit.max(0) == 0
    {
        crate::perf::push_elapsed(&mut timings, "total", total_start);
        return Ok(empty_report("", 0, timings));
    }
    let mut session = EmbeddingBackfillSession::start()?;
    reindex_memory_embeddings_with_session_report(conn, limit, &mut session)
}

pub(crate) fn reindex_memory_embeddings_with_session_report(
    conn: &Connection,
    limit: i64,
    session: &mut EmbeddingBackfillSession,
) -> Result<EmbeddingReindexReport> {
    let total_start = Instant::now();
    let mut timings = vec![];
    if session.is_disabled() {
        crate::perf::push_elapsed(&mut timings, "total", total_start);
        return Ok(empty_report("off", 0, timings));
    }
    if !super::table_exists(conn, "memories")?
        || !super::table_exists(conn, "memory_embeddings")?
        || limit.max(0) == 0
    {
        crate::perf::push_elapsed(&mut timings, "total", total_start);
        return Ok(empty_report("", 0, timings));
    }

    let profile_start = Instant::now();
    session.ensure_environment_unchanged("before selecting a backfill batch")?;
    crate::perf::push_elapsed(&mut timings, "profile_probe", profile_start);
    let target = session.target.clone();

    let select_start = Instant::now();
    let pending = select_memory_embedding_reindex_candidates(conn, &target, limit)?;
    crate::perf::push_elapsed(&mut timings, "select_pending", select_start);
    let selected = pending.len();
    if pending.is_empty() {
        session.ensure_environment_unchanged("before completing an empty backfill batch")?;
        crate::perf::push_elapsed(&mut timings, "total", total_start);
        return Ok(EmbeddingReindexReport {
            selected,
            processed: 0,
            model: target.model,
            dimensions: target.dimensions,
            timings,
        });
    }

    let prepared =
        prepare_memory_embedding_batch(&pending, &mut timings, &mut session.fallback_cache)?;
    session.observe_call_failure_fallback()?;
    validate_prepared_embedding_profiles(&target, &prepared)?;
    session.ensure_environment_unchanged("before writing a backfill batch")?;

    let processed = upsert_prepared_memory_embedding_batch(conn, &prepared, &mut timings)?;
    session.ensure_environment_unchanged("after writing a backfill batch")?;
    crate::perf::push_elapsed(&mut timings, "total", total_start);
    Ok(EmbeddingReindexReport {
        selected,
        processed,
        model: target.model,
        dimensions: target.dimensions,
        timings,
    })
}

fn validate_prepared_embedding_profiles(
    target: &EmbeddingBackfillTarget,
    prepared: &[PreparedMemoryEmbedding],
) -> Result<()> {
    for embedding in prepared {
        let actual_dimensions = embedding.values.len();
        if embedding.model != target.model || actual_dimensions != target.dimensions {
            anyhow::bail!(
                "pinned embedding profile changed while preparing memory id={}: expected model={} dimensions={}, got model={} dimensions={}; refusing to write mixed backfill batch",
                embedding.memory_id,
                target.model,
                target.dimensions,
                embedding.model,
                actual_dimensions
            );
        }
    }
    Ok(())
}

fn empty_report(
    model: &str,
    dimensions: usize,
    timings: Vec<crate::perf::PhaseTiming>,
) -> EmbeddingReindexReport {
    EmbeddingReindexReport {
        selected: 0,
        processed: 0,
        model: model.to_string(),
        dimensions,
        timings,
    }
}

pub fn pending_memory_embedding_count(conn: &Connection) -> Result<i64> {
    pending_memory_embedding_reindex_count(conn)
}

pub fn pending_memory_embedding_reindex_count(conn: &Connection) -> Result<i64> {
    if crate::retrieval::embedding::provider_disabled_or_error()? {
        return Ok(0);
    }
    if !super::table_exists(conn, "memories")? || !super::table_exists(conn, "memory_embeddings")? {
        return Ok(0);
    }
    let target = match crate::retrieval::embedding::configured_backfill_target() {
        Ok(target) => target,
        Err(error) if crate::retrieval::embedding::is_embedding_provider_off_error(&error) => {
            return Ok(0);
        }
        Err(error) => return Err(error),
    };
    pending_memory_embedding_reindex_count_for_target(conn, &target)
}

pub fn pending_memory_embedding_reindex_count_for_target(
    conn: &Connection,
    target: &EmbeddingBackfillTarget,
) -> Result<i64> {
    if target.dimensions == 0
        || !super::table_exists(conn, "memories")?
        || !super::table_exists(conn, "memory_embeddings")?
    {
        return Ok(0);
    }
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

fn upsert_prepared_memory_embedding_batch(
    conn: &Connection,
    prepared: &[PreparedMemoryEmbedding],
    timings: &mut Vec<crate::perf::PhaseTiming>,
) -> Result<usize> {
    if prepared.is_empty() {
        return Ok(0);
    }
    let prepared_count = prepared.len();
    conn.execute_batch("SAVEPOINT remem_embedding_reindex_batch")
        .context("start memory embedding reindex savepoint")?;
    let result = (|| -> Result<()> {
        let upsert_start = Instant::now();
        {
            let mut stmt = conn.prepare(super::UPSERT_EMBEDDING_SQL)?;
            for embedding in prepared {
                super::execute_embedding_upsert(
                    &mut stmt,
                    embedding.memory_id,
                    &embedding.model,
                    &embedding.content_hash,
                    &embedding.values,
                    embedding.updated_at_epoch,
                )
                .with_context(|| {
                    format!(
                        "memory embedding upsert failed for memory id={}",
                        embedding.memory_id
                    )
                })?;
            }
        }
        let mut by_dimensions: std::collections::BTreeMap<usize, Vec<i64>> =
            std::collections::BTreeMap::new();
        for embedding in prepared {
            by_dimensions
                .entry(embedding.values.len())
                .or_default()
                .push(embedding.memory_id);
        }
        for (dimensions, memory_ids) in by_dimensions {
            super::vec_index::sync_vec_upsert_batch(conn, dimensions, &memory_ids)?;
        }
        crate::perf::push_elapsed(timings, "upsert_embeddings", upsert_start);
        Ok(())
    })();

    match result {
        Ok(()) => {
            let commit_start = Instant::now();
            conn.execute_batch("RELEASE SAVEPOINT remem_embedding_reindex_batch")
                .context("release memory embedding reindex savepoint")?;
            crate::perf::push_elapsed(timings, "commit", commit_start);
            Ok(prepared_count)
        }
        Err(error) => {
            let rollback_result = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT remem_embedding_reindex_batch;
                 RELEASE SAVEPOINT remem_embedding_reindex_batch",
            );
            match rollback_result {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(error).context(format!(
                    "memory embedding reindex failed and rollback failed: {rollback_error}"
                )),
            }
        }
    }
}
