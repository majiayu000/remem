use anyhow::{Context, Result};
use rusqlite::{params, Connection};

const REAL_EMBEDDING_SIMILARITY_THRESHOLD: f32 = 0.95;
const LOCAL_FEATURE_HASH_SIMILARITY_THRESHOLD: f32 = 0.875;
const CANDIDATE_LIMIT: i64 = 200;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SemanticDuplicate {
    pub memory_id: i64,
    pub similarity: f32,
}

pub(crate) fn find_curated_duplicate_id(
    conn: &Connection,
    project: &str,
    scope: &str,
    memory_type: &str,
    title: &str,
    content: &str,
    topic_key: Option<&str>,
    branch: Option<&str>,
    now_epoch: i64,
) -> Result<Option<i64>> {
    Ok(find_curated_duplicate(
        conn,
        project,
        scope,
        memory_type,
        title,
        content,
        topic_key,
        branch,
        now_epoch,
    )?
    .map(|duplicate| duplicate.memory_id))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn find_curated_duplicate(
    conn: &Connection,
    project: &str,
    scope: &str,
    memory_type: &str,
    title: &str,
    content: &str,
    topic_key: Option<&str>,
    branch: Option<&str>,
    now_epoch: i64,
) -> Result<Option<SemanticDuplicate>> {
    if !is_curated_dedup_type(memory_type) || crate::retrieval::vector::embedding_count(conn)? == 0
    {
        return Ok(None);
    }
    if crate::retrieval::embedding::embedding_provider_status()?.disabled {
        return Ok(None);
    }

    let scope = if scope.trim().is_empty() {
        "project"
    } else {
        scope
    };
    let (owner_scope, owner_key) = crate::memory::operation::owner_for_scope(project, scope);
    let branch_key = branch.unwrap_or_default();
    let query_embedding =
        crate::retrieval::embedding::embed_memory(title, content, memory_type, topic_key)?;
    let threshold = similarity_threshold(query_embedding.model());
    let max_distance = 1.0 - threshold;
    let current_filter = crate::memory::memory_state_key_current_filter_sql("m");
    let sql = format!(
        "SELECT m.id, e.embedding, e.dimensions
         FROM memories m
         JOIN memory_embeddings e ON e.memory_id = m.id
         WHERE m.memory_type = ?1
           AND m.status = 'active'
           AND (m.expires_at_epoch IS NULL OR m.expires_at_epoch > ?2)
           AND {current_filter}
           AND COALESCE(m.scope, 'project') = ?3
           AND COALESCE(m.owner_scope, ?4) = ?4
           AND COALESCE(
                 m.owner_key,
                 CASE
                   WHEN COALESCE(m.scope, 'project') = 'global' THEN 'user:default'
                   ELSE m.project
                 END
               ) = ?5
           AND COALESCE(m.branch, '') = ?6
           AND e.model = ?7
           AND e.dimensions = ?8
         ORDER BY m.updated_at_epoch DESC, m.id DESC
         LIMIT ?9"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![
            memory_type,
            now_epoch,
            scope,
            owner_scope,
            owner_key,
            branch_key,
            query_embedding.model(),
            query_embedding.dimensions() as i64,
            CANDIDATE_LIMIT
        ],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
            ))
        },
    )?;
    let candidates = crate::db::query::collect_rows(rows)?;
    let mut best: Option<SemanticDuplicate> = None;
    for (memory_id, blob, dimensions) in candidates {
        let stored = crate::retrieval::vector::decode_embedding(&blob, dimensions)
            .with_context(|| format!("invalid embedding blob for memory id={memory_id}"))?;
        let distance = crate::retrieval::vector::cosine_distance(query_embedding.values(), &stored)
            .with_context(|| format!("compare semantic duplicate candidate id={memory_id}"))?;
        if distance > max_distance {
            continue;
        }
        let similarity = 1.0 - distance;
        match &best {
            Some(current) if current.similarity >= similarity => {}
            _ => {
                best = Some(SemanticDuplicate {
                    memory_id,
                    similarity,
                });
            }
        }
    }
    Ok(best)
}

fn is_curated_dedup_type(memory_type: &str) -> bool {
    matches!(memory_type, "preference" | "lesson")
}

fn similarity_threshold(model: &str) -> f32 {
    if model == crate::retrieval::embedding::LOCAL_EMBEDDING_MODEL {
        LOCAL_FEATURE_HASH_SIMILARITY_THRESHOLD
    } else {
        REAL_EMBEDDING_SIMILARITY_THRESHOLD
    }
}
