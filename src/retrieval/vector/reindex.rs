use anyhow::Result;
use rusqlite::{params, Connection};

use super::MemoryEmbeddingReindexCandidate;

pub(super) fn select_memory_embedding_reindex_candidates(
    conn: &Connection,
    target: &crate::retrieval::embedding::EmbeddingBackfillTarget,
    limit: i64,
) -> Result<Vec<MemoryEmbeddingReindexCandidate>> {
    let sql = "SELECT m.id, m.topic_key, m.title, m.content, m.memory_type
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
            })
        },
    )?;
    crate::db::query::collect_rows(rows)
}
