use anyhow::{Context, Result};
use rusqlite::{params, Connection, Row, Statement};
use std::time::Instant;

#[derive(Clone)]
pub(super) struct MemoryEmbeddingReindexCandidate {
    pub(super) id: i64,
    pub(super) topic_key: Option<String>,
    pub(super) title: String,
    pub(super) content: String,
    pub(super) memory_type: String,
    pub(super) search_context: String,
}

impl MemoryEmbeddingReindexCandidate {
    fn index_hash(&self) -> String {
        crate::retrieval::embedding::memory_index_hash(
            &self.title,
            &self.content,
            &self.memory_type,
            self.topic_key.as_deref(),
            &self.search_context,
        )
    }
}

pub(super) struct PreparedMemoryEmbedding {
    pub(super) source: MemoryEmbeddingReindexCandidate,
    pub(super) model: String,
    pub(super) content_hash: String,
    pub(super) values: Vec<f32>,
    pub(super) updated_at_epoch: i64,
}

// Timestamp ordering is only selection priority. Freshness is established by
// hashing the actual passage, including the effective enrichment snapshot.
const SELECT_MEMORY_EMBEDDING_SOURCES_SQL: &str =
    "SELECT m.id, m.topic_key, m.title, m.content, m.memory_type,
            CASE WHEN m.search_context_source_hash IS NOT NULL
                 THEN COALESCE(m.search_context, '') ELSE '' END,
            e.content_hash
     FROM memories m
     LEFT JOIN memory_embeddings e
       ON e.memory_id = m.id AND e.model = ?1
      AND (?2 IS NULL OR e.dimensions = ?2)
     WHERE m.status IN ('active', 'stale', 'archived')
     ORDER BY m.updated_at_epoch DESC, m.id DESC";

pub(super) const UPSERT_CURRENT_EMBEDDING_SQL: &str = "INSERT INTO memory_embeddings
         (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
     SELECT m.id, ?2, ?3, ?4, ?5, ?6 FROM memories m
     WHERE m.id = ?1 AND m.status IN ('active', 'stale', 'archived')
       AND m.topic_key IS ?7 AND m.title = ?8 AND m.content = ?9
       AND m.memory_type = ?10
       AND (CASE WHEN m.search_context_source_hash IS NOT NULL
                 THEN COALESCE(m.search_context, '') ELSE '' END) = ?11
     ON CONFLICT(memory_id, model, dimensions) DO UPDATE SET
         embedding = excluded.embedding,
         content_hash = excluded.content_hash,
         updated_at_epoch = excluded.updated_at_epoch";

fn source_from_row(row: &Row<'_>) -> rusqlite::Result<MemoryEmbeddingReindexCandidate> {
    Ok(MemoryEmbeddingReindexCandidate {
        id: row.get(0)?,
        topic_key: row.get(1)?,
        title: row.get(2)?,
        content: row.get(3)?,
        memory_type: row.get(4)?,
        search_context: row.get(5)?,
    })
}

/// Read-only, streaming consistency scan. Counts can scan all eligible passage
/// bytes; they perform no model calls, writes, or timestamp-based shortcuts.
pub(super) fn memory_embedding_source_counts(
    conn: &Connection,
    model: &str,
    dimensions: Option<usize>,
) -> Result<(i64, i64)> {
    let mut stmt = conn.prepare(SELECT_MEMORY_EMBEDDING_SOURCES_SQL)?;
    let mut rows = stmt.query(params![model, dimensions.map(|value| value as i64)])?;
    let (mut total, mut fresh) = (0, 0);
    let mut previous_id = None;
    let mut previous_fresh = false;
    while let Some(row) = rows.next()? {
        let source = source_from_row(row)?;
        if previous_id != Some(source.id) {
            total += 1;
            previous_id = Some(source.id);
            previous_fresh = false;
        }
        let stored_hash: Option<String> = row.get(6)?;
        if !previous_fresh && stored_hash.as_deref() == Some(source.index_hash().as_str()) {
            fresh += 1;
            previous_fresh = true;
        }
    }
    Ok((total, fresh))
}

pub(super) fn select_memory_embedding_reindex_candidates(
    conn: &Connection,
    target: &crate::retrieval::embedding::EmbeddingBackfillTarget,
    limit: i64,
) -> Result<Vec<MemoryEmbeddingReindexCandidate>> {
    let mut selected = Vec::new();
    if limit <= 0 {
        return Ok(selected);
    }
    let mut stmt = conn.prepare(SELECT_MEMORY_EMBEDDING_SOURCES_SQL)?;
    let mut rows = stmt.query(params![target.model.as_str(), target.dimensions as i64])?;
    while let Some(row) = rows.next()? {
        let source = source_from_row(row)?;
        let stored_hash: Option<String> = row.get(6)?;
        if stored_hash.as_deref() != Some(source.index_hash().as_str()) {
            selected.push(source);
            if selected.len() as i64 >= limit {
                break;
            }
        }
    }
    Ok(selected)
}

pub(super) fn execute_prepared_embedding_upsert(
    stmt: &mut Statement<'_>,
    embedding: &PreparedMemoryEmbedding,
) -> Result<bool> {
    super::validate_embedding(&embedding.model, &embedding.values)?;
    let source = &embedding.source;
    let written = stmt.execute(params![
        source.id,
        super::encode_embedding(&embedding.values),
        embedding.values.len() as i64,
        embedding.model,
        embedding.content_hash,
        embedding.updated_at_epoch,
        source.topic_key,
        source.title,
        source.content,
        source.memory_type,
        source.search_context,
    ])?;
    Ok(written != 0)
}

pub(super) fn prepare_memory_embedding_batch(
    batch: &[MemoryEmbeddingReindexCandidate],
    timings: &mut Vec<crate::perf::PhaseTiming>,
    fallback_cache: &mut crate::retrieval::embedding::EmbeddingFallbackCache,
) -> Result<Vec<PreparedMemoryEmbedding>> {
    if batch.is_empty() {
        return Ok(Vec::new());
    }

    let embed_start = Instant::now();
    let mut prepared = Vec::with_capacity(batch.len());
    for candidate in batch {
        prepared.push(
            prepare_memory_embedding(candidate, fallback_cache).with_context(|| {
                format!(
                    "memory embedding preparation failed for memory id={}",
                    candidate.id
                )
            })?,
        );
    }
    crate::perf::push_elapsed(timings, "embed_memory", embed_start);
    Ok(prepared)
}

fn prepare_memory_embedding(
    candidate: &MemoryEmbeddingReindexCandidate,
    fallback_cache: &mut crate::retrieval::embedding::EmbeddingFallbackCache,
) -> Result<PreparedMemoryEmbedding> {
    let embedding = crate::retrieval::embedding::embed_memory_index_with_fallback_cache(
        &candidate.title,
        &candidate.content,
        &candidate.memory_type,
        candidate.topic_key.as_deref(),
        &candidate.search_context,
        fallback_cache,
    )?;
    let content_hash = candidate.index_hash();
    Ok(PreparedMemoryEmbedding {
        source: candidate.clone(),
        model: embedding.model().to_string(),
        content_hash,
        values: embedding.values().to_vec(),
        updated_at_epoch: chrono::Utc::now().timestamp(),
    })
}
