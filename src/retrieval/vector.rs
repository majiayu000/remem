use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Statement};

use super::embedding::TextEmbedding;
pub use super::vector_candidates::VECTOR_SEARCH_CANDIDATE_LIMIT;

mod backfill;
mod coverage;
mod reindex;
mod vec_index;

pub(crate) use vec_index::ensure_vec_index;

pub use super::embedding::{
    LOCAL_EMBEDDING_DIMENSIONS as EMBEDDING_DIMENSIONS,
    LOCAL_EMBEDDING_MODEL as DEFAULT_EMBEDDING_MODEL,
};
pub use backfill::{
    backfill_missing_memory_embeddings, pending_memory_embedding_count,
    pending_memory_embedding_reindex_count, pending_memory_embedding_reindex_count_for_target,
    reindex_memory_embeddings, reindex_memory_embeddings_with_report, EmbeddingReindexReport,
};
pub(crate) use backfill::{
    reindex_memory_embeddings_with_session_report, EmbeddingBackfillSession,
};
pub use coverage::{
    active_embedding_coverage, active_embedding_coverage_for_status,
    active_embedding_coverage_for_target, prune_inactive_memory_embeddings,
    ActiveEmbeddingCoverage, InactiveEmbeddingPruneReport,
};

const EMBEDDING_REINDEX_WRITE_BATCH_SIZE: usize = 512;
const UPSERT_EMBEDDING_SQL: &str = "INSERT INTO memory_embeddings
         (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(memory_id, model, dimensions) DO UPDATE SET
             embedding = excluded.embedding,
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

/// Register the statically linked sqlite-vec extension on this connection
/// (GH-957). The crate compiles `sqlite-vec.c` with `SQLITE_CORE`, so direct
/// initialization with a null API pointer is the supported pattern and no
/// dynamic loading is involved. An init failure logs at error level and
/// leaves the connection on the portable brute-force cosine scan; the search
/// contract is unchanged either way.
pub fn load_vec_extension(conn: &Connection) -> Result<()> {
    if vec_extension_loaded(conn) {
        return Ok(());
    }
    // The crate declares the entry point with a zero-arg signature (intended
    // for transmute into `sqlite3_auto_extension`, which would only serve
    // connections opened after registration). Rebind the same symbol with the
    // true SQLite extension entry-point signature so every connection is
    // initialized deterministically at open time.
    type SqliteVecInit = unsafe extern "C" fn(
        db: *mut rusqlite::ffi::sqlite3,
        pz_err_msg: *mut *mut std::ffi::c_char,
        p_api: *const std::ffi::c_void,
    ) -> std::ffi::c_int;
    // SAFETY: `sqlite-vec` is compiled with SQLITE_CORE (see its build.rs), so
    // the p_api argument is unused and null is valid; the transmute only
    // restores the entry point's real signature (the crate's own test uses the
    // same address for auto-extension registration). The handle comes from a
    // live rusqlite connection on this thread.
    let init: SqliteVecInit =
        unsafe { std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ()) };
    let rc = unsafe { init(conn.handle(), std::ptr::null_mut(), std::ptr::null()) };
    if rc != rusqlite::ffi::SQLITE_OK {
        crate::log::error(
            "retrieval",
            &format!(
                "sqlite-vec init failed with rc={rc}; vector index disabled, brute-force scan remains"
            ),
        );
        return Ok(());
    }
    if !vec_extension_loaded(conn) {
        crate::log::error(
            "retrieval",
            "sqlite-vec init reported success but vec_version() is unavailable; brute-force scan remains",
        );
    }
    Ok(())
}

/// True when sqlite-vec functions answer on this connection.
pub(crate) fn vec_extension_loaded(conn: &Connection) -> bool {
    conn.query_row("SELECT vec_version()", [], |row| row.get::<_, String>(0))
        .is_ok()
}

pub fn ensure_vec_table(conn: &Connection) -> Result<()> {
    create_embedding_table(conn)
}

pub fn upsert_embedding(conn: &Connection, memory_id: i64, embedding: &[f32]) -> Result<()> {
    if super::embedding::provider_disabled_or_error()? {
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

/// Upsert the index embedding for one memory row from the authoritative
/// passage (canonical fields + index-only `search_context`). The stored
/// `content_hash` is the versioned `memory_index_hash` of the same snapshot.
pub fn upsert_memory_embedding(
    conn: &Connection,
    memory_id: i64,
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
    search_context: &str,
) -> Result<()> {
    if super::embedding::provider_disabled_or_error()? {
        return Ok(());
    }
    let embedding = match super::embedding::embed_memory_index(
        title,
        content,
        memory_type,
        topic_key,
        search_context,
    ) {
        Ok(embedding) => embedding,
        Err(error) if super::embedding::is_embedding_provider_off_error(&error) => return Ok(()),
        Err(error) if super::embedding::is_local_embedding_model_unavailable_error(&error) => {
            crate::log::error(
                "embedding",
                &format!("memory embedding deferred for memory id={memory_id}: {error}"),
            );
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let content_hash =
        super::embedding::memory_index_hash(title, content, memory_type, topic_key, search_context);
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

/// Row-atomic upsert used by the enrichment success CAS: the caller already
/// prepared the vector and index hash for the committed snapshot.
pub(crate) fn upsert_index_embedding(
    conn: &Connection,
    memory_id: i64,
    model: &str,
    index_hash: &str,
    values: &[f32],
) -> Result<()> {
    upsert_embedding_with_metadata(
        conn,
        memory_id,
        model,
        index_hash,
        values,
        chrono::Utc::now().timestamp(),
    )
    .with_context(|| format!("index embedding upsert failed for memory id={memory_id}"))
}

pub fn upsert_memory_embedding_for_row(conn: &Connection, memory_id: i64) -> Result<()> {
    let row: (Option<String>, String, String, String, Option<String>) = conn
        .query_row(
            "SELECT topic_key, title, content, memory_type,
                    CASE WHEN search_context_source_hash IS NOT NULL
                         THEN search_context ELSE '' END
             FROM memories
             WHERE id = ?1",
            [memory_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .with_context(|| format!("load memory row for embedding id={memory_id}"))?;
    let (topic_key, title, content, memory_type, search_context) = row;
    upsert_memory_embedding(
        conn,
        memory_id,
        &title,
        &content,
        &memory_type,
        topic_key.as_deref(),
        search_context.as_deref().unwrap_or(""),
    )
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
    crate::memory::retrieval_enrichment::ensure_retrieval_open(conn)?;
    if super::embedding::provider_disabled_or_error()? {
        return Ok(VectorSearchOutcome::disabled("embedding provider is off"));
    }
    if !table_exists(conn, "memory_embeddings")? {
        return Ok(VectorSearchOutcome::disabled(
            "memory_embeddings table is missing; run migrations/backfill",
        ));
    }
    let mut timings = Vec::new();
    let profile = query_embedding.profile();
    let knn_hits = crate::perf::time_result(&mut timings, "vector_knn_index", || {
        vec_index::knn_candidates(
            conn,
            query_embedding.values(),
            profile,
            filters,
            super::vector_candidates::vector_candidate_limit(limit),
        )
    })?;
    if let Some(mut hits) = knn_hits {
        // An empty KNN answer falls through to the portable path so the
        // caller still gets its empty-store / missing-profile diagnostics.
        if !hits.is_empty() {
            let candidates_scanned = hits.len();
            hits.truncate(limit);
            return Ok(VectorSearchOutcome::ready_with_scan_count_and_timings(
                hits,
                candidates_scanned,
                timings,
            ));
        }
    }
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
             memory_id INTEGER NOT NULL,
             embedding BLOB NOT NULL,
             dimensions INTEGER NOT NULL,
             model TEXT NOT NULL,
             content_hash TEXT NOT NULL,
             updated_at_epoch INTEGER NOT NULL,
             PRIMARY KEY(memory_id, model, dimensions),
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
    )?;
    vec_index::sync_vec_upsert(conn, memory_id, model, embedding.len())
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
        .as_chunks::<{ std::mem::size_of::<f32>() }>()
        .0
        .iter()
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
