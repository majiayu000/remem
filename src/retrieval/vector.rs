use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Statement};
use std::time::Instant;

use super::embedding::TextEmbedding;
pub use super::vector_candidates::VECTOR_SEARCH_CANDIDATE_LIMIT;

mod coverage;

pub use super::embedding::{
    LOCAL_EMBEDDING_DIMENSIONS as EMBEDDING_DIMENSIONS,
    LOCAL_EMBEDDING_MODEL as DEFAULT_EMBEDDING_MODEL,
};
pub use coverage::{
    active_embedding_coverage, active_embedding_coverage_for_status, ActiveEmbeddingCoverage,
};

const EMBEDDING_REINDEX_WRITE_BATCH_SIZE: usize = 512;
const UPSERT_EMBEDDING_SQL: &str = "INSERT INTO memory_embeddings
         (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(memory_id) DO UPDATE SET
             embedding = excluded.embedding,
             dimensions = excluded.dimensions,
             model = excluded.model,
             content_hash = excluded.content_hash,
             updated_at_epoch = excluded.updated_at_epoch";

#[derive(Debug, Clone, PartialEq)]
pub struct VectorHit {
    pub memory_id: i64,
    pub distance: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorSearchOutcome {
    pub hits: Vec<VectorHit>,
    pub disabled_reason: Option<String>,
    pub candidates_scanned: usize,
    pub timings: Vec<crate::perf::PhaseTiming>,
}

impl VectorSearchOutcome {
    pub fn disabled(reason: impl Into<String>) -> Self {
        Self {
            hits: vec![],
            disabled_reason: Some(reason.into()),
            candidates_scanned: 0,
            timings: vec![],
        }
    }

    fn disabled_with_timings(
        reason: impl Into<String>,
        timings: Vec<crate::perf::PhaseTiming>,
    ) -> Self {
        Self {
            hits: vec![],
            disabled_reason: Some(reason.into()),
            candidates_scanned: 0,
            timings,
        }
    }

    pub fn ready(hits: Vec<VectorHit>) -> Self {
        let candidates_scanned = hits.len();
        Self::ready_with_scan_count(hits, candidates_scanned)
    }

    pub fn ready_with_scan_count(hits: Vec<VectorHit>, candidates_scanned: usize) -> Self {
        Self::ready_with_scan_count_and_timings(hits, candidates_scanned, vec![])
    }

    fn ready_with_scan_count_and_timings(
        hits: Vec<VectorHit>,
        candidates_scanned: usize,
        timings: Vec<crate::perf::PhaseTiming>,
    ) -> Self {
        Self {
            hits,
            disabled_reason: None,
            candidates_scanned,
            timings,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct VectorSearchFilters<'a> {
    pub project: Option<&'a str>,
    pub memory_type: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub include_stale: bool,
}

/// Load a native vector extension when one is configured.
///
/// The current production path is a portable SQLite table plus in-process
/// cosine scan. That keeps vector recall available in the single-binary build;
/// sqlite-vec can replace the scan later without changing the search contract.
pub fn load_vec_extension(_conn: &Connection) -> Result<()> {
    Ok(())
}

pub fn ensure_vec_table(conn: &Connection) -> Result<()> {
    create_embedding_table(conn)
}

pub fn upsert_embedding(conn: &Connection, memory_id: i64, embedding: &[f32]) -> Result<()> {
    if super::embedding::embedding_provider_status_without_probe()?.disabled {
        return Ok(());
    }
    upsert_embedding_with_metadata(
        conn,
        memory_id,
        DEFAULT_EMBEDDING_MODEL,
        "",
        embedding,
        chrono::Utc::now().timestamp(),
    )
}

pub fn upsert_memory_embedding(
    conn: &Connection,
    memory_id: i64,
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
) -> Result<()> {
    if super::embedding::embedding_provider_status_without_probe()?.disabled {
        return Ok(());
    }
    let embedding = match super::embedding::embed_memory(title, content, memory_type, topic_key) {
        Ok(embedding) => embedding,
        Err(error) if super::embedding::is_embedding_provider_off_error(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    let content_hash =
        super::embedding::embedding_content_hash(title, content, memory_type, topic_key);
    upsert_embedding_with_metadata(
        conn,
        memory_id,
        embedding.model(),
        &content_hash,
        embedding.values(),
        chrono::Utc::now().timestamp(),
    )
    .with_context(|| format!("memory embedding upsert failed for memory id={memory_id}"))
}

pub fn upsert_memory_embedding_for_row(conn: &Connection, memory_id: i64) -> Result<()> {
    let (topic_key, title, content, memory_type): (Option<String>, String, String, String) = conn
        .query_row(
            "SELECT topic_key, title, content, memory_type
             FROM memories
             WHERE id = ?1",
            [memory_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .with_context(|| format!("load memory row for embedding id={memory_id}"))?;
    upsert_memory_embedding(
        conn,
        memory_id,
        &title,
        &content,
        &memory_type,
        topic_key.as_deref(),
    )
}

pub fn backfill_missing_memory_embeddings(conn: &Connection, limit: i64) -> Result<usize> {
    reindex_memory_embeddings(conn, limit)
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingReindexReport {
    pub selected: usize,
    pub processed: usize,
    pub model: String,
    pub dimensions: usize,
    pub timings: Vec<crate::perf::PhaseTiming>,
}

pub fn reindex_memory_embeddings(conn: &Connection, limit: i64) -> Result<usize> {
    let mut remaining_limit = limit.max(0);
    let mut processed = 0usize;
    while remaining_limit > 0 {
        let batch_limit = remaining_limit.min(EMBEDDING_REINDEX_WRITE_BATCH_SIZE as i64);
        let report = reindex_memory_embeddings_with_report(conn, batch_limit)?;
        if report.processed == 0 {
            break;
        }
        processed += report.processed;
        remaining_limit -= report.processed as i64;
        if report.processed < batch_limit as usize {
            break;
        }
    }
    Ok(processed)
}

pub fn reindex_memory_embeddings_with_report(
    conn: &Connection,
    limit: i64,
) -> Result<EmbeddingReindexReport> {
    let total_start = Instant::now();
    let mut timings = vec![];
    if super::embedding::embedding_provider_status_without_probe()?.disabled {
        crate::perf::push_elapsed(&mut timings, "total", total_start);
        return Ok(EmbeddingReindexReport {
            selected: 0,
            processed: 0,
            model: "off".to_string(),
            dimensions: 0,
            timings,
        });
    }
    if !table_exists(conn, "memories")? || !table_exists(conn, "memory_embeddings")? {
        crate::perf::push_elapsed(&mut timings, "total", total_start);
        return Ok(EmbeddingReindexReport {
            selected: 0,
            processed: 0,
            model: String::new(),
            dimensions: 0,
            timings,
        });
    }
    let limit = limit.max(0);
    if limit == 0 {
        crate::perf::push_elapsed(&mut timings, "total", total_start);
        return Ok(EmbeddingReindexReport {
            selected: 0,
            processed: 0,
            model: String::new(),
            dimensions: 0,
            timings,
        });
    }

    let profile_start = Instant::now();
    let target = match super::embedding::configured_backfill_target() {
        Ok(target) => target,
        Err(error) if super::embedding::is_embedding_provider_off_error(&error) => {
            crate::perf::push_elapsed(&mut timings, "total", total_start);
            return Ok(EmbeddingReindexReport {
                selected: 0,
                processed: 0,
                model: "off".to_string(),
                dimensions: 0,
                timings,
            });
        }
        Err(error) => return Err(error),
    };
    crate::perf::push_elapsed(&mut timings, "profile_probe", profile_start);

    let select_start = Instant::now();
    let sql = "SELECT m.id, m.topic_key, m.title, m.content, m.memory_type
         FROM memories m
         LEFT JOIN memory_embeddings e ON e.memory_id = m.id
         WHERE (e.memory_id IS NULL
                OR e.model <> ?1
                OR e.dimensions <> ?2
                OR e.updated_at_epoch < m.updated_at_epoch)
           AND m.status IN ('active', 'stale', 'archived')
         ORDER BY m.updated_at_epoch DESC, m.id DESC
         LIMIT ?3";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(
        params![target.model.as_str(), target.dimensions as i64, limit],
        |row| {
            Ok(MemoryEmbeddingReindexCandidate {
                id: row.get(0)?,
                topic_key: row.get(1)?,
                title: row.get(2)?,
                content: row.get(3)?,
                memory_type: row.get(4)?,
            })
        },
    )?;
    let pending = crate::db::query::collect_rows(rows)?;
    crate::perf::push_elapsed(&mut timings, "select_pending", select_start);

    let selected = pending.len();
    if pending.is_empty() {
        crate::perf::push_elapsed(&mut timings, "total", total_start);
        return Ok(EmbeddingReindexReport {
            selected,
            processed: 0,
            model: target.model,
            dimensions: target.dimensions,
            timings,
        });
    }

    let processed = reindex_memory_embedding_batch(conn, &pending, &mut timings)?;
    crate::perf::push_elapsed(&mut timings, "total", total_start);
    Ok(EmbeddingReindexReport {
        selected,
        processed,
        model: target.model,
        dimensions: target.dimensions,
        timings,
    })
}

pub fn pending_memory_embedding_count(conn: &Connection) -> Result<i64> {
    if !table_exists(conn, "memories")? || !table_exists(conn, "memory_embeddings")? {
        return Ok(0);
    }
    count_pending_memory_embedding_reindex(conn)
}

pub fn pending_memory_embedding_reindex_count(conn: &Connection) -> Result<i64> {
    count_pending_memory_embedding_reindex(conn)
}

pub fn embedding_count(conn: &Connection) -> Result<i64> {
    if !table_exists(conn, "memory_embeddings")? {
        return Ok(0);
    }
    Ok(
        conn.query_row("SELECT COUNT(*) FROM memory_embeddings", [], |row| {
            row.get(0)
        })?,
    )
}

struct MemoryEmbeddingReindexCandidate {
    id: i64,
    topic_key: Option<String>,
    title: String,
    content: String,
    memory_type: String,
}

struct PreparedMemoryEmbedding {
    memory_id: i64,
    model: String,
    content_hash: String,
    values: Vec<f32>,
    updated_at_epoch: i64,
}

fn reindex_memory_embedding_batch(
    conn: &Connection,
    batch: &[MemoryEmbeddingReindexCandidate],
    timings: &mut Vec<crate::perf::PhaseTiming>,
) -> Result<usize> {
    if batch.is_empty() {
        return Ok(0);
    }

    let embed_start = Instant::now();
    let mut prepared = Vec::with_capacity(batch.len());
    for candidate in batch {
        prepared.push(prepare_memory_embedding(candidate).with_context(|| {
            format!(
                "memory embedding preparation failed for memory id={}",
                candidate.id
            )
        })?);
    }
    let prepared_count = prepared.len();
    crate::perf::push_elapsed(timings, "embed_memory", embed_start);

    conn.execute_batch("SAVEPOINT remem_embedding_reindex_batch")
        .context("start memory embedding reindex savepoint")?;
    let result = (|| -> Result<()> {
        let upsert_start = Instant::now();
        {
            let mut stmt = conn.prepare(UPSERT_EMBEDDING_SQL)?;
            for embedding in &prepared {
                execute_embedding_upsert(
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

fn prepare_memory_embedding(
    candidate: &MemoryEmbeddingReindexCandidate,
) -> Result<PreparedMemoryEmbedding> {
    let embedding = super::embedding::embed_memory(
        &candidate.title,
        &candidate.content,
        &candidate.memory_type,
        candidate.topic_key.as_deref(),
    )?;
    let content_hash = super::embedding::embedding_content_hash(
        &candidate.title,
        &candidate.content,
        &candidate.memory_type,
        candidate.topic_key.as_deref(),
    );
    Ok(PreparedMemoryEmbedding {
        memory_id: candidate.id,
        model: embedding.model().to_string(),
        content_hash,
        values: embedding.values().to_vec(),
        updated_at_epoch: chrono::Utc::now().timestamp(),
    })
}

fn count_pending_memory_embedding_reindex(conn: &Connection) -> Result<i64> {
    if super::embedding::embedding_provider_status_without_probe()?.disabled {
        return Ok(0);
    }
    if !table_exists(conn, "memories")? || !table_exists(conn, "memory_embeddings")? {
        return Ok(0);
    }
    let target = match super::embedding::configured_backfill_target() {
        Ok(target) => target,
        Err(error) if super::embedding::is_embedding_provider_off_error(&error) => return Ok(0),
        Err(error) => return Err(error),
    };
    let sql = "SELECT COUNT(*)
               FROM memories m
               LEFT JOIN memory_embeddings e ON e.memory_id = m.id
               WHERE (e.memory_id IS NULL
                      OR e.model <> ?1
                      OR e.dimensions <> ?2
                      OR e.updated_at_epoch < m.updated_at_epoch)
                 AND m.status IN ('active', 'stale', 'archived')";
    Ok(conn.query_row(
        sql,
        params![target.model.as_str(), target.dimensions as i64],
        |row| row.get(0),
    )?)
}

pub fn embed_query_text(query: &str) -> Vec<f32> {
    super::embedding::embed_query_text_local(query)
}

pub fn embed_memory_text(
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
) -> Vec<f32> {
    super::embedding::embed_memory_text_local(title, content, memory_type, topic_key)
}

pub fn vector_search(
    conn: &Connection,
    query_embedding: &[f32],
    limit: usize,
) -> Result<Vec<(i64, f32)>> {
    Ok(
        vector_search_filtered(conn, query_embedding, VectorSearchFilters::default(), limit)?
            .hits
            .into_iter()
            .map(|hit| (hit.memory_id, hit.distance))
            .collect(),
    )
}

pub fn vector_search_filtered(
    conn: &Connection,
    query_embedding: &[f32],
    filters: VectorSearchFilters<'_>,
    limit: usize,
) -> Result<VectorSearchOutcome> {
    if query_embedding.len() != EMBEDDING_DIMENSIONS {
        anyhow::bail!(
            "query embedding must be {} dimensions, got {}",
            EMBEDDING_DIMENSIONS,
            query_embedding.len()
        );
    }
    let embedding = TextEmbedding::new(DEFAULT_EMBEDDING_MODEL, query_embedding.to_vec())?;
    vector_search_embedding_filtered(conn, &embedding, filters, limit)
}

pub fn vector_search_embedding_filtered(
    conn: &Connection,
    query_embedding: &TextEmbedding,
    filters: VectorSearchFilters<'_>,
    limit: usize,
) -> Result<VectorSearchOutcome> {
    if limit == 0 {
        return Ok(VectorSearchOutcome::ready(vec![]));
    }
    if super::embedding::embedding_provider_status_without_probe()?.disabled {
        return Ok(VectorSearchOutcome::disabled("embedding provider is off"));
    }
    if !table_exists(conn, "memory_embeddings")? {
        return Ok(VectorSearchOutcome::disabled(
            "memory_embeddings table is missing; run migrations/backfill",
        ));
    }
    let mut timings = Vec::new();
    let profile = query_embedding.profile();
    let candidate_ids = crate::perf::time_result(&mut timings, "vector_select_candidates", || {
        super::vector_candidates::select_candidate_ids(conn, filters, profile, limit)
    })?;
    let candidates_scanned = candidate_ids.len();
    if candidate_ids.is_empty() {
        if super::vector_candidates::matching_memory_count(conn, filters)? > 0 {
            if embedding_count(conn)? == 0 {
                return Ok(VectorSearchOutcome::disabled_with_timings(
                    "memory_embeddings table is empty; run `remem reindex-embeddings --limit 1000`",
                    timings,
                ));
            }
            return Ok(VectorSearchOutcome::disabled_with_timings(
                format!(
                    "memory_embeddings has no rows for model={} dimensions={}; run `remem reindex-embeddings --limit 1000`",
                    profile.model, profile.dimensions
                ),
                timings,
            ));
        }
        return Ok(VectorSearchOutcome::ready_with_scan_count_and_timings(
            vec![],
            0,
            timings,
        ));
    }
    let placeholders = std::iter::repeat_n("?", candidate_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT memory_id, embedding, dimensions
         FROM memory_embeddings INDEXED BY idx_memory_embeddings_profile_memory_id
         WHERE model = ?
           AND dimensions = ?
           AND memory_id IN ({placeholders})"
    );
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(profile.model.to_string()),
        Box::new(profile.dimensions as i64),
    ];
    param_values.extend(
        candidate_ids
            .iter()
            .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>),
    );
    let candidates = crate::perf::time_result(&mut timings, "vector_load_embeddings", || {
        let refs = crate::db::to_sql_refs(&param_values);
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(refs.as_slice(), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        crate::db::query::collect_rows(rows)
    })?;
    let mut hits = crate::perf::time_result(&mut timings, "vector_decode_cosine", || {
        let mut hits = Vec::new();
        for (memory_id, blob, dimensions) in candidates {
            let embedding = decode_embedding(&blob, dimensions)
                .with_context(|| format!("invalid embedding blob for memory id={memory_id}"))?;
            let distance = cosine_distance(query_embedding.values(), &embedding)?;
            hits.push(VectorHit {
                memory_id,
                distance,
            });
        }
        Ok(hits)
    })?;
    crate::perf::time_value(&mut timings, "vector_sort_truncate", || {
        hits.sort_by(|a, b| {
            a.distance
                .partial_cmp(&b.distance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.memory_id.cmp(&b.memory_id))
        });
        hits.truncate(limit);
    });
    Ok(VectorSearchOutcome::ready_with_scan_count_and_timings(
        hits,
        candidates_scanned,
        timings,
    ))
}

pub fn find_similar_observations(
    conn: &Connection,
    query_embedding: &[f32],
    threshold: f32,
    limit: usize,
) -> Result<Vec<i64>> {
    let candidates = vector_search(conn, query_embedding, limit)?;
    let distance_threshold = 1.0 - threshold;
    let similar: Vec<i64> = candidates
        .into_iter()
        .filter(|(_, dist)| *dist < distance_threshold)
        .map(|(id, _)| id)
        .collect();

    Ok(similar)
}

fn create_embedding_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_embeddings (
             memory_id INTEGER PRIMARY KEY,
             embedding BLOB NOT NULL,
             dimensions INTEGER NOT NULL,
             model TEXT NOT NULL,
             content_hash TEXT NOT NULL,
             updated_at_epoch INTEGER NOT NULL,
             FOREIGN KEY(memory_id) REFERENCES memories(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_memory_embeddings_model
             ON memory_embeddings(model, updated_at_epoch);
         CREATE INDEX IF NOT EXISTS idx_memory_embeddings_profile_memory_id
             ON memory_embeddings(model, dimensions, memory_id);",
    )?;
    Ok(())
}

fn upsert_embedding_with_metadata(
    conn: &Connection,
    memory_id: i64,
    model: &str,
    content_hash: &str,
    embedding: &[f32],
    updated_at_epoch: i64,
) -> Result<()> {
    let mut stmt = conn.prepare(UPSERT_EMBEDDING_SQL)?;
    execute_embedding_upsert(
        &mut stmt,
        memory_id,
        model,
        content_hash,
        embedding,
        updated_at_epoch,
    )
}

fn execute_embedding_upsert(
    stmt: &mut Statement<'_>,
    memory_id: i64,
    model: &str,
    content_hash: &str,
    embedding: &[f32],
    updated_at_epoch: i64,
) -> Result<()> {
    if model.trim().is_empty() {
        anyhow::bail!("embedding model must not be empty");
    }
    if embedding.is_empty() {
        anyhow::bail!("embedding vector must not be empty");
    }
    if embedding.iter().any(|value| !value.is_finite()) {
        anyhow::bail!("embedding vector contains non-finite values");
    }
    let blob = encode_embedding(embedding);
    let dimensions = embedding.len() as i64;
    stmt.execute(params![
        memory_id,
        blob,
        dimensions,
        model,
        content_hash,
        updated_at_epoch
    ])?;
    Ok(())
}

fn encode_embedding(embedding: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(std::mem::size_of_val(embedding));
    for value in embedding {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

pub(crate) fn decode_embedding(blob: &[u8], dimensions: i64) -> Result<Vec<f32>> {
    if dimensions <= 0 {
        anyhow::bail!("embedding dimensions must be positive, got {dimensions}");
    }
    let dimensions = dimensions as usize;
    let expected_bytes = dimensions * std::mem::size_of::<f32>();
    if blob.len() != expected_bytes {
        anyhow::bail!(
            "embedding blob must be {} bytes, got {}",
            expected_bytes,
            blob.len()
        );
    }
    Ok(blob
        .chunks_exact(std::mem::size_of::<f32>())
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

pub(crate) fn cosine_distance(a: &[f32], b: &[f32]) -> Result<f32> {
    if a.len() != b.len() {
        anyhow::bail!(
            "embedding dimensions differ: query={} stored={}",
            a.len(),
            b.len()
        );
    }
    let mut dot = 0.0f32;
    let mut a_norm = 0.0f32;
    let mut b_norm = 0.0f32;
    for (left, right) in a.iter().zip(b) {
        dot += left * right;
        a_norm += left * left;
        b_norm += right * right;
    }
    if a_norm == 0.0 || b_norm == 0.0 {
        return Ok(1.0);
    }
    Ok((1.0 - dot / (a_norm.sqrt() * b_norm.sqrt())).clamp(0.0, 2.0))
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
            params![table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

#[cfg(test)]
mod tests;
