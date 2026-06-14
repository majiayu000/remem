use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use crate::memory::{self, Memory};
use crate::retrieval::search::common::{
    rank_normalized_score, sanitize_fts_query, weighted_ranked_fuse, WeightedRankedChannel,
    WeightedRankedHit,
};

use super::filters::{push_excluded_type_filter, push_owner_included_filter};

const RRF_K: f64 = 60.0;
const MAX_VECTOR_DISTANCE: f32 = 0.51;
const FTS_WEIGHT: f64 = 2.5;
const VECTOR_WEIGHT: f64 = 3.0;
const ENTITY_WEIGHT: f64 = 1.25;
const TEMPORAL_WEIGHT: f64 = 1.0;
const LIKE_FALLBACK_WEIGHT: f64 = 0.25;
const MIN_HYBRID_FETCH_LIMIT: i64 = 20;

struct ContextChannel {
    weight: f64,
    hits: Vec<WeightedRankedHit>,
}

pub(super) fn query_hybrid_context_memories(
    conn: &Connection,
    project: &str,
    query: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    limit: i64,
) -> Result<Vec<Memory>> {
    if limit <= 0 || query.trim().is_empty() {
        return Ok(vec![]);
    }

    let fetch_limit = limit.saturating_mul(3).max(MIN_HYBRID_FETCH_LIMIT);
    let mut channels = Vec::new();
    push_channel(
        &mut channels,
        FTS_WEIGHT,
        query_local_fts_channel(
            conn,
            project,
            query,
            current_branch,
            excluded_types,
            fetch_limit,
        )?,
    );
    push_channel(
        &mut channels,
        ENTITY_WEIGHT,
        query_local_entity_channel(
            conn,
            project,
            query,
            current_branch,
            excluded_types,
            fetch_limit,
        )?,
    );
    push_channel(
        &mut channels,
        TEMPORAL_WEIGHT,
        query_local_temporal_channel(
            conn,
            project,
            query,
            current_branch,
            excluded_types,
            fetch_limit,
        )?,
    );
    push_channel(
        &mut channels,
        VECTOR_WEIGHT,
        query_local_vector_channel(conn, project, query, current_branch, excluded_types)?,
    );

    if channels.is_empty() {
        push_channel(
            &mut channels,
            LIKE_FALLBACK_WEIGHT,
            query_local_like_channel(
                conn,
                project,
                query,
                current_branch,
                excluded_types,
                fetch_limit,
            )?,
        );
    }
    if channels.is_empty() {
        return Ok(vec![]);
    }

    let channel_inputs = channels
        .iter()
        .map(|channel| WeightedRankedChannel {
            weight: channel.weight,
            hits: &channel.hits,
        })
        .collect::<Vec<_>>();
    let ids = weighted_ranked_fuse(&channel_inputs, RRF_K)
        .into_iter()
        .take(limit as usize)
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    query_owner_included_memories_by_ids(conn, project, &ids, current_branch, excluded_types)
}

fn push_channel(channels: &mut Vec<ContextChannel>, weight: f64, hits: Vec<WeightedRankedHit>) {
    if !hits.is_empty() {
        channels.push(ContextChannel { weight, hits });
    }
}

fn query_owner_included_memories_by_ids(
    conn: &Connection,
    project: &str,
    ids: &[i64],
    current_branch: Option<&str>,
    excluded_types: &[&str],
) -> Result<Vec<Memory>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }

    let placeholders = (1..=ids.len())
        .map(|idx| format!("?{idx}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut conditions = vec![format!("id IN ({placeholders})")];
    let mut params = ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect::<Vec<_>>();
    let mut idx = ids.len() + 1;
    push_context_memory_filters(
        project,
        current_branch,
        excluded_types,
        "",
        &mut idx,
        &mut conditions,
        &mut params,
    );

    let sql = format!(
        "SELECT {} FROM memories WHERE {}",
        memory::MEMORY_COLS,
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), memory::map_memory_row_pub)?;
    let mut rows_by_id = crate::db::query::collect_rows(rows)?
        .into_iter()
        .map(|memory| (memory.id, memory))
        .collect::<std::collections::HashMap<_, _>>();
    Ok(ids.iter().filter_map(|id| rows_by_id.remove(id)).collect())
}

fn query_local_fts_channel(
    conn: &Connection,
    project: &str,
    query: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    limit: i64,
) -> Result<Vec<WeightedRankedHit>> {
    let expanded = crate::retrieval::query_expand::expand_query(query);
    let long_tokens = expanded
        .iter()
        .filter(|token| token.chars().count() >= 3)
        .map(String::as_str)
        .collect::<Vec<_>>();
    if long_tokens.is_empty() {
        return Ok(vec![]);
    }

    let safe_query = sanitize_fts_query(&long_tokens.join(" "));
    let mut conditions = vec!["memories_fts MATCH ?1".to_string()];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(safe_query)];
    let mut idx = 2;
    push_context_memory_filters(
        project,
        current_branch,
        excluded_types,
        "m",
        &mut idx,
        &mut conditions,
        &mut params,
    );
    params.push(Box::new(limit));

    let sql = format!(
        "SELECT m.id, bm25(memories_fts, 10.0, 1.0, 3.0) AS rank_score
         FROM memories m
         JOIN memories_fts ON memories_fts.rowid = m.id
         WHERE {}
         ORDER BY rank_score ASC, m.updated_at_epoch DESC, m.id ASC
         LIMIT ?{idx}",
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
    })?;
    let hits = crate::db::query::collect_rows(rows)?;
    Ok(fts_ranked_hits(&hits))
}

fn query_local_entity_channel(
    conn: &Connection,
    project: &str,
    query: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    limit: i64,
) -> Result<Vec<WeightedRankedHit>> {
    let entities = crate::retrieval::entity::extract_entities(query, "");
    if entities.is_empty() {
        query_local_entity_like_channel(conn, project, query, current_branch, excluded_types, limit)
    } else {
        query_local_entity_exact_channel(
            conn,
            project,
            &entities,
            current_branch,
            excluded_types,
            limit,
        )
    }
}

fn query_local_entity_exact_channel(
    conn: &Connection,
    project: &str,
    entities: &[String],
    current_branch: Option<&str>,
    excluded_types: &[&str],
    limit: i64,
) -> Result<Vec<WeightedRankedHit>> {
    let placeholders = (1..=entities.len())
        .map(|idx| format!("?{idx}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut conditions = vec![format!(
        "e.canonical_name COLLATE NOCASE IN ({placeholders})"
    )];
    let mut params = entities
        .iter()
        .map(|entity| Box::new(entity.to_string()) as Box<dyn rusqlite::types::ToSql>)
        .collect::<Vec<_>>();
    let mut idx = entities.len() + 1;
    query_local_entity_ids(
        conn,
        project,
        current_branch,
        excluded_types,
        limit,
        &mut idx,
        &mut conditions,
        &mut params,
    )
}

fn query_local_entity_like_channel(
    conn: &Connection,
    project: &str,
    query: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    limit: i64,
) -> Result<Vec<WeightedRankedHit>> {
    let terms = query
        .split_whitespace()
        .filter(|term| term.chars().count() >= 2)
        .take(8)
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return Ok(vec![]);
    }
    let mut conditions = Vec::new();
    let mut params = Vec::new();
    let mut like_clauses = Vec::new();
    let mut idx = 1;
    for term in terms {
        like_clauses.push(format!("e.canonical_name LIKE ?{idx} COLLATE NOCASE"));
        params.push(Box::new(format!("%{term}%")) as Box<dyn rusqlite::types::ToSql>);
        idx += 1;
    }
    conditions.push(format!("({})", like_clauses.join(" OR ")));
    query_local_entity_ids(
        conn,
        project,
        current_branch,
        excluded_types,
        limit,
        &mut idx,
        &mut conditions,
        &mut params,
    )
}

fn query_local_entity_ids(
    conn: &Connection,
    project: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    limit: i64,
    idx: &mut usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> Result<Vec<WeightedRankedHit>> {
    push_context_memory_filters(
        project,
        current_branch,
        excluded_types,
        "m",
        idx,
        conditions,
        params,
    );
    params.push(Box::new(limit));
    let sql = format!(
        "SELECT me.memory_id, COUNT(DISTINCT me.entity_id) AS shared_count
         FROM memory_entities me
         JOIN entities e ON e.id = me.entity_id
         JOIN memories m ON m.id = me.memory_id
         WHERE {}
         GROUP BY me.memory_id
         ORDER BY shared_count DESC, m.updated_at_epoch DESC, me.memory_id DESC
         LIMIT ?{idx}",
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(params);
    let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, i64>(0))?;
    Ok(rank_ordered_hits(crate::db::query::collect_rows(rows)?))
}

fn query_local_temporal_channel(
    conn: &Connection,
    project: &str,
    query: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    limit: i64,
) -> Result<Vec<WeightedRankedHit>> {
    let Some(constraint) = crate::retrieval::temporal::extract_temporal(query) else {
        return Ok(vec![]);
    };
    let has_memory_facts = sqlite_table_available(conn, "memory_facts")?;
    let has_memory_fact_invalidations =
        has_memory_facts && crate::memory::facts::invalidated_at_epoch_available(conn)?;
    let (temporal_condition, order_epoch) = local_temporal_sql(
        constraint.field,
        has_memory_facts,
        has_memory_fact_invalidations,
    );
    let mut conditions = vec![temporal_condition];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(constraint.start_epoch),
        Box::new(constraint.end_epoch),
    ];
    let mut idx = 3;
    push_context_memory_filters(
        project,
        current_branch,
        excluded_types,
        "m",
        &mut idx,
        &mut conditions,
        &mut params,
    );
    params.push(Box::new(limit));

    let sql = format!(
        "SELECT m.id
         FROM memories m
         WHERE {}
         ORDER BY {order_epoch} DESC, m.id DESC
         LIMIT ?{idx}",
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, i64>(0))?;
    Ok(rank_ordered_hits(crate::db::query::collect_rows(rows)?))
}

fn query_local_vector_channel(
    conn: &Connection,
    project: &str,
    query: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
) -> Result<Vec<WeightedRankedHit>> {
    if !sqlite_table_available(conn, "memory_embeddings")? {
        return Ok(vec![]);
    }

    let query_embedding = crate::retrieval::embedding::embed_query(query)?;
    let profile = query_embedding.profile();
    let mut conditions = vec!["e.model = ?1".to_string(), "e.dimensions = ?2".to_string()];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(profile.model.to_string()),
        Box::new(profile.dimensions as i64),
    ];
    let mut idx = 3;
    push_context_memory_filters(
        project,
        current_branch,
        excluded_types,
        "m",
        &mut idx,
        &mut conditions,
        &mut params,
    );
    params.push(Box::new(
        crate::retrieval::vector::VECTOR_SEARCH_CANDIDATE_LIMIT as i64,
    ));

    let sql = format!(
        "SELECT e.memory_id, e.embedding, e.dimensions
         FROM memory_embeddings e
         JOIN memories m ON m.id = e.memory_id
         WHERE {}
         ORDER BY m.updated_at_epoch DESC, e.memory_id DESC
         LIMIT ?{idx}",
        conditions.join(" AND ")
    );
    let refs = crate::db::to_sql_refs(&params);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut hits = Vec::new();
    for row in crate::db::query::collect_rows(rows)? {
        let (memory_id, blob, dimensions) = row;
        let embedding = crate::retrieval::vector::decode_embedding(&blob, dimensions)?;
        let distance =
            crate::retrieval::vector::cosine_distance(query_embedding.values(), &embedding)?;
        if distance <= MAX_VECTOR_DISTANCE {
            hits.push((memory_id, distance));
        }
    }
    hits.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    Ok(hits
        .into_iter()
        .enumerate()
        .map(|(rank, (id, distance))| WeightedRankedHit {
            id,
            normalized_score: vector_similarity_score(distance).max(rank_normalized_score(rank)),
        })
        .collect())
}

fn query_local_like_channel(
    conn: &Connection,
    project: &str,
    query: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    limit: i64,
) -> Result<Vec<WeightedRankedHit>> {
    let tokens = crate::retrieval::query_expand::core_tokens(query);
    if tokens.is_empty() {
        return Ok(vec![]);
    }

    let mut conditions = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;
    for token in &tokens {
        conditions.push(format!("(m.title LIKE ?{idx} OR m.content LIKE ?{idx})"));
        params.push(Box::new(format!("%{token}%")));
        idx += 1;
    }
    push_context_memory_filters(
        project,
        current_branch,
        excluded_types,
        "m",
        &mut idx,
        &mut conditions,
        &mut params,
    );
    params.push(Box::new(limit));

    let sql = format!(
        "SELECT m.id
         FROM memories m
         WHERE {}
         ORDER BY m.updated_at_epoch DESC, m.id DESC
         LIMIT ?{idx}",
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, i64>(0))?;
    Ok(rank_ordered_hits(crate::db::query::collect_rows(rows)?))
}

fn push_context_memory_filters(
    project: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    alias: &str,
    idx: &mut usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) {
    let status_col = qualify(alias, "status");
    let expires_col = qualify(alias, "expires_at_epoch");
    conditions.push(crate::memory::memory_current_filter_sql(
        &status_col,
        &expires_col,
        false,
    ));
    conditions.push(crate::memory::memory_state_key_current_filter_sql(alias));
    push_owner_included_filter(project, idx, conditions, params);
    if let Some(branch) = current_branch.filter(|branch| !branch.trim().is_empty()) {
        conditions.push(format!(
            "({}.branch = ?{idx} OR {}.branch IS NULL)",
            table_ref(alias),
            table_ref(alias)
        ));
        params.push(Box::new(branch.to_string()));
        *idx += 1;
    }
    push_excluded_type_filter(excluded_types, idx, conditions, params);
}

fn qualify(alias: &str, column: &str) -> String {
    if alias.trim().is_empty() {
        column.to_string()
    } else {
        format!("{}.{}", alias.trim(), column)
    }
}

fn table_ref(alias: &str) -> &str {
    if alias.trim().is_empty() {
        "memories"
    } else {
        alias.trim()
    }
}

fn local_temporal_sql(
    field: crate::retrieval::temporal::TemporalField,
    has_memory_facts: bool,
    has_memory_fact_invalidations: bool,
) -> (String, String) {
    match field {
        crate::retrieval::temporal::TemporalField::UpdatedAt => (
            "m.updated_at_epoch BETWEEN ?1 AND ?2".to_string(),
            "m.updated_at_epoch".to_string(),
        ),
        crate::retrieval::temporal::TemporalField::EventTime if has_memory_facts => {
            let current_fact_filter =
                crate::memory::facts::current_fact_filter_sql("f", has_memory_fact_invalidations);
            let fact_event_overlap = format!(
                "f.source_memory_id = m.id \
                 AND {current_fact_filter} \
                 AND f.valid_from_epoch IS NOT NULL \
                 AND f.valid_from_epoch <= ?2 \
                 AND (f.valid_to_epoch IS NULL OR f.valid_to_epoch > ?1)"
            );
            let any_fact_event = format!(
                "f.source_memory_id = m.id \
                 AND {current_fact_filter} \
                 AND f.valid_from_epoch IS NOT NULL"
            );
            (
                format!(
                    "(EXISTS (
                         SELECT 1 FROM memory_facts f
                         WHERE {fact_event_overlap}
                     )
                     OR (
                         NOT EXISTS (
                             SELECT 1 FROM memory_facts f
                             WHERE {any_fact_event}
                         )
                         AND m.created_at_epoch BETWEEN ?1 AND ?2
                     ))"
                ),
                format!(
                    "COALESCE((
                         SELECT MAX(f.valid_from_epoch)
                         FROM memory_facts f
                         WHERE {fact_event_overlap}
                     ), m.created_at_epoch)"
                ),
            )
        }
        crate::retrieval::temporal::TemporalField::EventTime => (
            "m.created_at_epoch BETWEEN ?1 AND ?2".to_string(),
            "m.created_at_epoch".to_string(),
        ),
    }
}

fn sqlite_table_available(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
            [table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn fts_ranked_hits(hits: &[(i64, f64)]) -> Vec<WeightedRankedHit> {
    let best = hits
        .iter()
        .map(|(_, score)| *score)
        .fold(f64::INFINITY, f64::min);
    let worst = hits
        .iter()
        .map(|(_, score)| *score)
        .fold(f64::NEG_INFINITY, f64::max);
    let spread = worst - best;
    hits.iter()
        .enumerate()
        .map(|(rank, (id, score))| WeightedRankedHit {
            id: *id,
            normalized_score: if spread.abs() < f64::EPSILON {
                rank_normalized_score(rank)
            } else {
                ((worst - *score) / spread).clamp(0.0, 1.0)
            },
        })
        .collect()
}

fn rank_ordered_hits(ids: Vec<i64>) -> Vec<WeightedRankedHit> {
    ids.into_iter()
        .enumerate()
        .map(|(rank, id)| WeightedRankedHit {
            id,
            normalized_score: rank_normalized_score(rank),
        })
        .collect()
}

fn vector_similarity_score(distance: f32) -> f64 {
    let threshold = f64::from(MAX_VECTOR_DISTANCE);
    ((threshold - f64::from(distance)) / threshold).clamp(0.0, 1.0)
}
