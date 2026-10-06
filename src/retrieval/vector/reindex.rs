use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::time::Instant;

pub(super) struct MemoryEmbeddingReindexCandidate {
    pub(super) id: i64,
    pub(super) topic_key: Option<String>,
    pub(super) title: String,
    pub(super) content: String,
    pub(super) memory_type: String,
    pub(super) search_context: String,
}

impl MemoryEmbeddingReindexCandidate {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            topic_key: row.get(1)?,
            title: row.get(2)?,
            content: row.get(3)?,
            memory_type: row.get(4)?,
            search_context: row.get(5)?,
        })
    }

    fn content_hash(&self) -> String {
        crate::retrieval::embedding::memory_index_hash(
            &self.title,
            &self.content,
            &self.memory_type,
            self.topic_key.as_deref(),
            &self.search_context,
        )
    }
}

// Hash every eligible passage: completion timestamps can be newer than an
// intervening source edit, and source edits can share the same timestamp.
// Only enrichment-ready rows include search_context, matching foreground
// writers and curated semantic-dedup comparisons.
const REINDEX_SOURCE_SQL: &str = "SELECT m.id, m.topic_key, m.title, m.content, m.memory_type,
            CASE WHEN m.search_context_source_hash IS NOT NULL
                 THEN COALESCE(m.search_context, '') ELSE '' END,
            e.content_hash
     FROM memories m
     LEFT JOIN memory_embeddings e
       ON e.memory_id = m.id
      AND e.model = ?1
      AND e.dimensions = ?2
     WHERE m.status IN ('active', 'stale', 'archived')";

pub(super) struct PreparedMemoryEmbedding {
    pub(super) memory_id: i64,
    pub(super) model: String,
    pub(super) content_hash: String,
    pub(super) values: Vec<f32>,
    pub(super) updated_at_epoch: i64,
}

pub(super) fn select_memory_embedding_reindex_candidates(
    conn: &Connection,
    target: &crate::retrieval::embedding::EmbeddingBackfillTarget,
    limit: i64,
) -> Result<Vec<MemoryEmbeddingReindexCandidate>> {
    if limit <= 0 {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(&format!(
        "{REINDEX_SOURCE_SQL} ORDER BY m.updated_at_epoch DESC, m.id DESC"
    ))?;
    let mut rows = stmt.query(params![target.model.as_str(), target.dimensions as i64])?;
    let mut pending = Vec::new();
    while let Some(row) = rows.next()? {
        let candidate = MemoryEmbeddingReindexCandidate::from_row(row)?;
        let stored_hash: Option<String> = row.get(6)?;
        if stored_hash.as_deref() != Some(candidate.content_hash().as_str()) {
            pending.push(candidate);
            if pending.len() as i64 >= limit {
                break;
            }
        }
    }
    Ok(pending)
}

pub(super) fn count_memory_embedding_reindex_candidates(
    conn: &Connection,
    target: &crate::retrieval::embedding::EmbeddingBackfillTarget,
) -> Result<i64> {
    let mut stmt = conn.prepare(REINDEX_SOURCE_SQL)?;
    let mut rows = stmt.query(params![target.model.as_str(), target.dimensions as i64])?;
    let mut pending = 0;
    while let Some(row) = rows.next()? {
        let candidate = MemoryEmbeddingReindexCandidate::from_row(row)?;
        let stored_hash: Option<String> = row.get(6)?;
        if stored_hash.as_deref() != Some(candidate.content_hash().as_str()) {
            pending += 1;
        }
    }
    Ok(pending)
}

// The caller must hold the same transaction for this comparison and the
// embedding/mirror writes. SQLite rejects an invalidated read snapshot on
// upgrade rather than letting an intervening writer bypass this comparison.
pub(super) fn prepared_memory_source_is_current(
    conn: &Connection,
    prepared: &PreparedMemoryEmbedding,
) -> Result<bool> {
    let current = conn
        .query_row(
            "SELECT id, topic_key, title, content, memory_type,
                    CASE WHEN search_context_source_hash IS NOT NULL
                         THEN COALESCE(search_context, '') ELSE '' END
             FROM memories
             WHERE id = ?1 AND status IN ('active', 'stale', 'archived')",
            [prepared.memory_id],
            MemoryEmbeddingReindexCandidate::from_row,
        )
        .optional()?;
    Ok(current.is_some_and(|source| source.content_hash() == prepared.content_hash))
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
    let content_hash = candidate.content_hash();
    Ok(PreparedMemoryEmbedding {
        memory_id: candidate.id,
        model: embedding.model().to_string(),
        content_hash,
        values: embedding.values().to_vec(),
        updated_at_epoch: chrono::Utc::now().timestamp(),
    })
}
