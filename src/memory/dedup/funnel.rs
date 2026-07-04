use std::collections::BTreeSet;

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

pub(crate) fn check_duplicate_texts(narrative: &str, candidate_texts: &[String]) -> Result<bool> {
    if candidate_texts.is_empty() {
        return Ok(false);
    }
    let content_hash = crate::db::content_identity_hash(narrative.as_bytes());
    if candidate_texts.iter().any(|candidate| {
        let candidate_hash = crate::db::content_identity_hash(candidate.as_bytes());
        let legacy_candidate_hash = crate::db::legacy_content_identity_hash(candidate.as_bytes());
        candidate_hash == content_hash || legacy_candidate_hash == content_hash
    }) {
        return Ok(true);
    }

    let duplicate_indexes = find_vector_duplicate_indexes(
        narrative,
        candidate_texts
            .iter()
            .enumerate()
            .map(|(index, text)| (index, text.as_str())),
    )?;
    Ok(!duplicate_indexes.is_empty())
}

fn find_vector_duplicates(
    conn: &Connection,
    project: &str,
    narrative: &str,
    window_secs: i64,
) -> Result<Vec<i64>> {
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
    let candidates = candidates
        .into_iter()
        .filter_map(|(id, text, candidate_narrative, title, facts)| {
            canonical_observation_text(
                text.as_deref(),
                candidate_narrative.as_deref(),
                title.as_deref(),
                facts.as_deref(),
            )
            .map(|candidate_text| (id, candidate_text))
        })
        .collect::<Vec<_>>();
    let duplicate_indexes = find_vector_duplicate_indexes(
        narrative,
        candidates
            .iter()
            .enumerate()
            .map(|(index, (_id, text))| (index, text.as_str())),
    )?;
    Ok(duplicate_indexes
        .into_iter()
        .map(|index| candidates[index].0)
        .collect())
}

fn find_vector_duplicate_indexes<'a>(
    narrative: &str,
    candidates: impl IntoIterator<Item = (usize, &'a str)>,
) -> Result<Vec<usize>> {
    let candidates = candidates
        .into_iter()
        .filter(|(_index, text)| !text.trim().is_empty())
        .filter(|(_index, text)| !observation_text_conflicts(narrative, text))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut fallback_cache = crate::retrieval::embedding::EmbeddingFallbackCache::default();
    let mut query_embedding = match embed_observation_dedup_text(narrative, &mut fallback_cache) {
        Ok(embedding) => embedding,
        Err(error) if crate::retrieval::embedding::is_embedding_provider_off_error(&error) => {
            return Ok(Vec::new());
        }
        Err(error) => return Err(error),
    };
    let mut threshold = observation_similarity_threshold(query_embedding.model());
    let mut max_distance = 1.0 - threshold;
    let mut duplicates = Vec::new();
    for (index, candidate_text) in candidates {
        let candidate_embedding =
            match embed_observation_dedup_text(candidate_text, &mut fallback_cache) {
                Ok(embedding) => embedding,
                Err(error)
                    if crate::retrieval::embedding::is_embedding_provider_off_error(&error) =>
                {
                    return Ok(Vec::new());
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("embed observation duplicate candidate index={index}")
                    });
                }
            };
        if candidate_embedding.model() != query_embedding.model()
            || candidate_embedding.dimensions() != query_embedding.dimensions()
        {
            query_embedding = match embed_observation_dedup_text(narrative, &mut fallback_cache) {
                Ok(embedding) => embedding,
                Err(error)
                    if crate::retrieval::embedding::is_embedding_provider_off_error(&error) =>
                {
                    return Ok(Vec::new());
                }
                Err(error) => {
                    return Err(error)
                        .context("re-embed observation query after provider fallback");
                }
            };
            threshold = observation_similarity_threshold(query_embedding.model());
            max_distance = 1.0 - threshold;
        }
        if candidate_embedding.model() != query_embedding.model()
            || candidate_embedding.dimensions() != query_embedding.dimensions()
        {
            continue;
        }
        let distance = crate::retrieval::vector::cosine_distance(
            query_embedding.values(),
            candidate_embedding.values(),
        )
        .with_context(|| format!("compare observation duplicate candidate index={index}"))?;
        if distance <= max_distance {
            duplicates.push(index);
        }
    }
    Ok(duplicates)
}

fn embed_observation_dedup_text(
    text: &str,
    fallback_cache: &mut crate::retrieval::embedding::EmbeddingFallbackCache,
) -> Result<crate::retrieval::embedding::TextEmbedding> {
    crate::retrieval::embedding::embed_query_with_fallback_cache(text, fallback_cache)
}

fn observation_similarity_threshold(model: &str) -> f32 {
    if model == crate::retrieval::embedding::FEATURE_HASH_EMBEDDING_MODEL {
        FEATURE_HASH_OBSERVATION_THRESHOLD
    } else {
        REAL_EMBEDDING_OBSERVATION_THRESHOLD
    }
}

fn observation_text_conflicts(incoming: &str, existing: &str) -> bool {
    let incoming_tokens = observation_token_list(incoming);
    let existing_tokens = observation_token_list(existing);
    let incoming = observation_token_set(&incoming_tokens);
    let existing = observation_token_set(&existing_tokens);
    const OPPOSITES: &[(&[&str], &[&str])] = &[
        (
            &[
                "pass",
                "passed",
                "passes",
                "passing",
                "success",
                "succeeded",
                "successful",
            ],
            &["fail", "failed", "fails", "failing", "failure", "broken"],
        ),
        (
            &["enable", "enabled", "enables", "on", "active"],
            &["disable", "disabled", "disables", "off", "inactive"],
        ),
        (
            &["allow", "allowed", "allows", "accept", "accepted"],
            &["block", "blocked", "blocks", "reject", "rejected"],
        ),
        (
            &["increase", "increased", "higher"],
            &["decrease", "decreased", "lower"],
        ),
        (&["true", "yes", "present"], &["false", "no", "absent"]),
    ];
    OPPOSITES.iter().any(|(left, right)| {
        (has_any(&incoming, left) && has_any(&existing, right))
            || (has_any(&incoming, right) && has_any(&existing, left))
            || negated_same_status_conflict(
                &incoming_tokens,
                &incoming,
                &existing_tokens,
                &existing,
                left,
            )
            || negated_same_status_conflict(
                &incoming_tokens,
                &incoming,
                &existing_tokens,
                &existing,
                right,
            )
    })
}

fn observation_token_list(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn observation_token_set(tokens: &[String]) -> BTreeSet<String> {
    tokens.iter().cloned().collect()
}

fn has_any(tokens: &BTreeSet<String>, values: &[&str]) -> bool {
    values.iter().any(|value| tokens.contains(*value))
}

fn negated_same_status_conflict(
    incoming_tokens: &[String],
    incoming: &BTreeSet<String>,
    existing_tokens: &[String],
    existing: &BTreeSet<String>,
    values: &[&str],
) -> bool {
    (has_any(incoming, values) && has_negated_any(existing_tokens, values))
        || (has_negated_any(incoming_tokens, values) && has_any(existing, values))
}

fn has_negated_any(tokens: &[String], values: &[&str]) -> bool {
    tokens.iter().enumerate().any(|(index, token)| {
        values.contains(&token.as_str()) && {
            let start = index.saturating_sub(3);
            tokens[start..index]
                .iter()
                .any(|candidate| is_negation_token(candidate))
        }
    })
}

fn is_negation_token(token: &str) -> bool {
    matches!(
        token,
        "not"
            | "never"
            | "no"
            | "without"
            | "cannot"
            | "cant"
            | "don"
            | "doesn"
            | "didn"
            | "isn"
            | "wasn"
            | "weren"
            | "won"
    )
}
