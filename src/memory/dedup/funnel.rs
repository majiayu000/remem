use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::memory::dedup::{
    canonical_observation_text, find_hash_duplicates, mark_duplicate_accessed,
};

const VECTOR_WINDOW_SECS: i64 = 15 * 60;
const VECTOR_CANDIDATE_LIMIT: i64 = 200;
const FEATURE_HASH_OBSERVATION_THRESHOLD: f32 = 0.82;
const REAL_EMBEDDING_OBSERVATION_THRESHOLD: f32 = 0.92;

/// Check if new observation is a duplicate using three-layer funnel.
/// Returns Some(duplicate_id) if duplicate found, None if unique.
pub fn check_duplicate(
    conn: &Connection,
    project: &str,
    narrative: &str,
    _embedding: Option<&[f32]>,
) -> Result<Option<i64>> {
    let content_hash = crate::db::content_identity_hash(narrative.as_bytes());

    let hash_dups = find_hash_duplicates(conn, project, &content_hash, VECTOR_WINDOW_SECS)?;
    if !hash_dups.is_empty() {
        crate::log::info(
            "dedup",
            &format!("hash duplicate found: {} matches", hash_dups.len()),
        );
        mark_duplicate_accessed(conn, &hash_dups)?;
        return Ok(Some(hash_dups[0]));
    }

    let vector_dups = find_vector_duplicates(conn, project, narrative, VECTOR_WINDOW_SECS)?;
    if !vector_dups.is_empty() {
        crate::log::info(
            "dedup",
            &format!("vector duplicate found: {} matches", vector_dups.len()),
        );
        mark_duplicate_accessed(conn, &vector_dups)?;
        return Ok(Some(vector_dups[0]));
    }

    // Layer 3: LLM judgment

    Ok(None)
}

fn find_vector_duplicates(
    conn: &Connection,
    project: &str,
    narrative: &str,
    window_secs: i64,
) -> Result<Vec<i64>> {
    let query_embedding = match crate::retrieval::embedding::embed_memory(
        "Observation",
        narrative,
        "observation",
        None,
    ) {
        Ok(embedding) => embedding,
        Err(error) if crate::retrieval::embedding::is_embedding_provider_off_error(&error) => {
            return Ok(Vec::new());
        }
        Err(error) => return Err(error),
    };
    let threshold = observation_similarity_threshold(query_embedding.model());
    let max_distance = 1.0 - threshold;
    let cutoff = chrono::Utc::now().timestamp() - window_secs;
    let mut stmt = conn.prepare(
        "SELECT id, text, narrative, title, facts
         FROM observations
         WHERE project = ?1
           AND status = 'active'
           AND created_at_epoch > ?2
           AND (
             (text IS NOT NULL AND length(text) > 0)
             OR (narrative IS NOT NULL AND length(narrative) > 0)
             OR (title IS NOT NULL AND length(title) > 0)
             OR (facts IS NOT NULL AND length(facts) > 0)
           )
         ORDER BY created_at_epoch DESC, id DESC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![project, cutoff, VECTOR_CANDIDATE_LIMIT], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    let candidates = crate::db::query::collect_rows(rows)?;
    let mut duplicates = Vec::new();
    for (id, text, narrative, title, facts) in candidates {
        let Some(candidate_text) = canonical_observation_text(
            text.as_deref(),
            narrative.as_deref(),
            title.as_deref(),
            facts.as_deref(),
        ) else {
            continue;
        };
        let candidate_embedding = crate::retrieval::embedding::embed_memory(
            "Observation",
            &candidate_text,
            "observation",
            None,
        )
        .with_context(|| format!("embed observation duplicate candidate id={id}"))?;
        if candidate_embedding.model() != query_embedding.model()
            || candidate_embedding.dimensions() != query_embedding.dimensions()
        {
            continue;
        }
        let distance = crate::retrieval::vector::cosine_distance(
            query_embedding.values(),
            candidate_embedding.values(),
        )
        .with_context(|| format!("compare observation duplicate candidate id={id}"))?;
        if distance <= max_distance {
            duplicates.push(id);
        }
    }
    Ok(duplicates)
}

fn observation_similarity_threshold(model: &str) -> f32 {
    if model == crate::retrieval::embedding::FEATURE_HASH_EMBEDDING_MODEL {
        FEATURE_HASH_OBSERVATION_THRESHOLD
    } else {
        REAL_EMBEDDING_OBSERVATION_THRESHOLD
    }
}
