//! sqlite-vec KNN index mirroring `memory_embeddings` (GH-957).
//!
//! The portable candidate path (`vector_candidates`) bounds the brute-force
//! cosine scan by sampling at most 4096 embeddings, so memories outside the
//! sampled id buckets are unreachable for the semantic channel no matter how
//! relevant they are. This module maintains one `vec0` virtual table per
//! embedding dimension profile (`memory_embedding_vec_{dimensions}`) and lets
//! the vector channel ask sqlite-vec for the globally nearest candidates
//! instead — relevance truncation instead of recency/bucket truncation.
//!
//! Invariants:
//! - The index is a pure derived mirror: `memory_embeddings` stays the source
//!   of truth, and every writer dual-writes here (upsert/delete).
//! - Everything degrades to the brute-force path: no extension on the
//!   connection, no vec table, or an incomplete backfill all answer `None`.
//! - Backfill is chunked (one batch per connection open) so a hook-triggered
//!   `open_db` never stalls on re-indexing a large store.

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::super::embedding::EmbeddingProfile;
use super::super::vector_candidates::memory_filter_conditions;
use super::{vec_extension_loaded, VectorHit, VectorSearchFilters};

pub(crate) const VEC_INDEX_BACKFILL_BATCH_SIZE: usize = 512;

const STATE_TABLE: &str = "memory_embedding_vec_state";

/// One sync-state row per dimension profile. `last_memory_id` is the backfill
/// cursor over `memory_embeddings` (ascending id order); `done` flips to 1
/// when a short batch proves the cursor passed the last row. Live writers
/// dual-write regardless of cursor position, so a partially backfilled table
/// is always a subset of the source plus newer dual-written rows.
fn ensure_state_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {STATE_TABLE} (
             dimensions INTEGER PRIMARY KEY,
             last_memory_id INTEGER NOT NULL DEFAULT 0,
             done INTEGER NOT NULL DEFAULT 0,
             updated_at_epoch INTEGER NOT NULL
         )"
    ))?;
    Ok(())
}

fn vec_table_name(dimensions: usize) -> String {
    format!("memory_embedding_vec_{dimensions}")
}

fn vec_table_exists(conn: &Connection, dimensions: usize) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [vec_table_name(dimensions)],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn create_vec_table(conn: &Connection, dimensions: usize) -> Result<()> {
    anyhow::ensure!(
        (1..=65_536).contains(&dimensions),
        "embedding dimensions out of indexable range: {dimensions}"
    );
    // `dimensions` is a validated integer, so identifier interpolation is safe.
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS {} USING vec0(
             memory_id INTEGER PRIMARY KEY,
             embedding float[{dimensions}] distance_metric=cosine,
             +model TEXT
         )",
        vec_table_name(dimensions)
    ))?;
    Ok(())
}

/// Advance the vec index by at most one backfill batch for every dimension
/// profile present in `memory_embeddings`. Cheap when in sync: one indexed
/// state-table lookup per profile.
pub(crate) fn ensure_vec_index(conn: &Connection) -> Result<()> {
    if !vec_extension_loaded(conn) {
        return Ok(());
    }
    ensure_state_table(conn)?;

    let mut profiles: Vec<i64> = {
        let mut stmt =
            conn.prepare("SELECT DISTINCT dimensions FROM memory_embeddings ORDER BY dimensions")?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        crate::db::query::collect_rows(rows)?
    };
    let builtin = super::EMBEDDING_DIMENSIONS as i64;
    if !profiles.contains(&builtin) {
        profiles.push(builtin);
    }

    for dimensions in profiles {
        anyhow::ensure!(
            dimensions > 0,
            "memory_embeddings carries non-positive dimensions {dimensions}"
        );
        ensure_vec_index_profile(conn, dimensions as usize)?;
    }
    Ok(())
}

fn ensure_vec_index_profile(conn: &Connection, dimensions: usize) -> Result<()> {
    let state: Option<(i64, i64)> = conn
        .query_row(
            &format!("SELECT last_memory_id, done FROM {STATE_TABLE} WHERE dimensions = ?1"),
            [dimensions as i64],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    let (cursor, done) = state.unwrap_or((0, 0));
    if done == 1 {
        return Ok(());
    }
    create_vec_table(conn, dimensions)?;

    let table = vec_table_name(dimensions);
    let mut stmt = conn.prepare(&format!(
        "SELECT memory_id, embedding, model
         FROM memory_embeddings
         WHERE dimensions = ?1 AND memory_id > ?2
         ORDER BY memory_id
         LIMIT ?3"
    ))?;
    let rows = stmt.query_map(
        rusqlite::params![
            dimensions as i64,
            cursor,
            VEC_INDEX_BACKFILL_BATCH_SIZE as i64
        ],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    )?;
    let batch = crate::db::query::collect_rows(rows)?;
    let mut insert = conn.prepare(&format!(
        "INSERT OR REPLACE INTO {table} (memory_id, embedding, model) VALUES (?1, ?2, ?3)"
    ))?;
    let mut advanced = cursor;
    for (memory_id, embedding, model) in &batch {
        insert.execute(rusqlite::params![memory_id, embedding, model])?;
        advanced = *memory_id;
    }
    let finished = (batch.len() < VEC_INDEX_BACKFILL_BATCH_SIZE) as i64;
    conn.execute(
        &format!(
            "INSERT INTO {STATE_TABLE} (dimensions, last_memory_id, done, updated_at_epoch)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(dimensions) DO UPDATE SET
                 last_memory_id = excluded.last_memory_id,
                 done = excluded.done,
                 updated_at_epoch = excluded.updated_at_epoch"
        ),
        rusqlite::params![
            dimensions as i64,
            advanced,
            finished,
            chrono::Utc::now().timestamp()
        ],
    )?;
    Ok(())
}

/// True when the KNN path can serve this profile on this connection: the
/// extension answered, the vec table exists, and its backfill completed.
pub(crate) fn vec_index_ready(conn: &Connection, profile: EmbeddingProfile<'_>) -> Result<bool> {
    if !vec_extension_loaded(conn) || !vec_table_exists(conn, profile.dimensions)? {
        return Ok(false);
    }
    let done: Option<i64> = conn
        .query_row(
            &format!("SELECT done FROM {STATE_TABLE} WHERE dimensions = ?1"),
            [profile.dimensions as i64],
            |row| row.get(0),
        )
        .ok();
    Ok(done == Some(1))
}

/// Mirror one `memory_embeddings` row into its vec table. No-op when the
/// extension or table is absent; the next backfill pass reconciles.
pub(crate) fn sync_vec_upsert(
    conn: &Connection,
    memory_id: i64,
    model: &str,
    dimensions: usize,
) -> Result<()> {
    if !vec_extension_loaded(conn) || !vec_table_exists(conn, dimensions)? {
        return Ok(());
    }
    conn.execute(
        &format!(
            "INSERT OR REPLACE INTO {} (memory_id, embedding, model)
             SELECT memory_id, embedding, model
             FROM memory_embeddings
             WHERE memory_id = ?1 AND model = ?2 AND dimensions = ?3",
            vec_table_name(dimensions)
        ),
        rusqlite::params![memory_id, model, dimensions as i64],
    )
    .with_context(|| format!("sync vec index for memory id={memory_id}"))?;
    Ok(())
}

/// Mirror a batch of `memory_embeddings` rows (one dimension profile) into
/// its vec table. Used by the reindex batch path, which writes through a
/// shared prepared statement instead of the single-row funnel.
pub(crate) fn sync_vec_upsert_batch(
    conn: &Connection,
    dimensions: usize,
    memory_ids: &[i64],
) -> Result<()> {
    if memory_ids.is_empty() || !vec_extension_loaded(conn) || !vec_table_exists(conn, dimensions)?
    {
        return Ok(());
    }
    let placeholders = memory_ids
        .iter()
        .enumerate()
        .map(|(index, _)| format!("?{}", index + 2))
        .collect::<Vec<_>>()
        .join(", ");
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(dimensions as i64)];
    values.extend(
        memory_ids
            .iter()
            .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>),
    );
    let refs = crate::db::to_sql_refs(&values);
    conn.execute(
        &format!(
            "INSERT OR REPLACE INTO {} (memory_id, embedding, model)
             SELECT memory_id, embedding, model
             FROM memory_embeddings
             WHERE dimensions = ?1 AND memory_id IN ({placeholders})",
            vec_table_name(dimensions)
        ),
        refs.as_slice(),
    )?;
    Ok(())
}

/// Mirror the inactive-profile prune on `memory_embeddings`: drop vec tables
/// for other dimension profiles (and their backfill state) and remove
/// non-target models from the surviving table.
pub(crate) fn sync_vec_keep_only_profile(
    conn: &Connection,
    model: &str,
    dimensions: usize,
) -> Result<()> {
    if !vec_extension_loaded(conn) {
        return Ok(());
    }
    let target_table = vec_table_name(dimensions);
    for name in existing_vec_tables(conn)? {
        if name == target_table {
            conn.execute(
                &format!("DELETE FROM \"{name}\" WHERE model != ?1"),
                [model],
            )?;
        } else {
            conn.execute_batch(&format!("DROP TABLE \"{name}\""))?;
        }
    }
    ensure_state_table(conn)?;
    conn.execute(
        &format!("DELETE FROM {STATE_TABLE} WHERE dimensions != ?1"),
        [dimensions as i64],
    )?;
    Ok(())
}

fn existing_vec_tables(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master
         WHERE type = 'table' AND name LIKE 'memory_embedding_vec_%'",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    // vec0 creates shadow tables (`_info`, `_chunks`, ...) under the same
    // prefix; only names whose suffix is purely the dimension number are the
    // virtual tables themselves.
    Ok(crate::db::query::collect_rows(rows)?
        .into_iter()
        .filter(|name| {
            name.strip_prefix("memory_embedding_vec_")
                .is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit())
                })
        })
        .collect())
}

/// Globally nearest candidates for the query embedding, or `None` when the
/// index cannot serve this profile (caller falls back to the brute-force
/// candidate path). Distances are sqlite-vec cosine distances, ascending, and
/// rows pass the same `memories` visibility filters as the portable path.
pub(crate) fn knn_candidates(
    conn: &Connection,
    query_embedding: &[f32],
    profile: EmbeddingProfile<'_>,
    filters: VectorSearchFilters<'_>,
    candidate_limit: usize,
) -> Result<Option<Vec<VectorHit>>> {
    if !vec_index_ready(conn, profile)? {
        return Ok(None);
    }
    anyhow::ensure!(
        query_embedding.len() == profile.dimensions,
        "query embedding must be {} dimensions, got {}",
        profile.dimensions,
        query_embedding.len()
    );
    let mut blob = Vec::with_capacity(std::mem::size_of_val(query_embedding));
    for value in query_embedding {
        blob.extend_from_slice(&value.to_le_bytes());
    }
    let (conditions, filter_values) = memory_filter_conditions(filters, 5);
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(blob),
        Box::new(candidate_limit as i64),
        Box::new(profile.model.to_string()),
        Box::new(profile.dimensions as i64),
    ];
    values.extend(filter_values);
    // sqlite-vec forbids WHERE constraints on vec0 auxiliary columns inside a
    // KNN query, so the model check goes through the source-of-truth table
    // instead of the mirrored `+model` column.
    let sql = format!(
        "SELECT v.memory_id, v.distance
         FROM {} v
         JOIN memories m ON m.id = v.memory_id
         WHERE v.embedding MATCH ?1 AND k = ?2
           AND EXISTS (
               SELECT 1 FROM memory_embeddings e
               WHERE e.memory_id = v.memory_id AND e.model = ?3 AND e.dimensions = ?4
           )
           AND {}
         ORDER BY v.distance",
        vec_table_name(profile.dimensions),
        conditions.join(" AND ")
    );
    let refs = crate::db::to_sql_refs(&values);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, f32>(1)?))
    })?;
    let hits = crate::db::query::collect_rows(rows)?
        .into_iter()
        .map(|(memory_id, distance)| VectorHit {
            memory_id,
            distance,
        })
        .collect::<Vec<_>>();
    Ok(Some(hits))
}
