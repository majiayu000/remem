use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::time::Instant;

pub(super) struct MemoryEmbeddingReindexCandidate {
    pub(super) id: i64,
    pub(super) topic_key: Option<String>,
    pub(super) title: String,
    pub(super) content: String,
    pub(super) memory_type: String,
    pub(super) search_context: String,
}

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
    // Only enrichment-ready rows embed the search_context snapshot; pending
    // rows embed the canonical passage so backfill matches the foreground
    // writers and curated semantic-dedup comparisons.
    let sql = "SELECT m.id, m.topic_key, m.title, m.content, m.memory_type,
                CASE WHEN m.search_context_source_hash IS NOT NULL
                     THEN COALESCE(m.search_context, '') ELSE '' END
         FROM memories m
         LEFT JOIN memory_embeddings e
           ON e.memory_id = m.id
          AND e.model = ?1
          AND e.dimensions = ?2
         WHERE (e.memory_id IS NULL
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
                search_context: row.get(5)?,
            })
        },
    )?;
    crate::db::query::collect_rows(rows)
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
    let content_hash = crate::retrieval::embedding::memory_index_hash(
        &candidate.title,
        &candidate.content,
        &candidate.memory_type,
        candidate.topic_key.as_deref(),
        &candidate.search_context,
    );
    Ok(PreparedMemoryEmbedding {
        memory_id: candidate.id,
        model: embedding.model().to_string(),
        content_hash,
        values: embedding.values().to_vec(),
        updated_at_epoch: chrono::Utc::now().timestamp(),
    })
}
