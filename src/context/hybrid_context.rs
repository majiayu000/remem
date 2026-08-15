use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use crate::memory;
use crate::retrieval::search::common::{
    calibrated_vector_hits, sanitize_fts_query, WeightedRankedHit,
};
use crate::retrieval::search::SearchWeights;

use super::filters::{push_excluded_type_filter, push_owner_included_filter};

mod query;
mod rank;
mod usage;

pub(crate) use query::query_hybrid_context_memories_with_rank_signal_mode;
#[cfg(test)]
pub(super) use query::query_owner_included_memories_by_ids;
pub(super) use query::{query_hybrid_context_memories, query_hybrid_context_memories_with_weights};

use rank::{fts_ranked_hits, rank_ordered_hits};

/// Injection-only: how deep each channel reads before fusion. Not a scoring
/// knob, so it stays here rather than in [`SearchWeights`] (GH953).
const MIN_HYBRID_FETCH_LIMIT: i64 = 20;

struct ContextChannel {
    weight: f64,
    hits: Vec<WeightedRankedHit>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum InjectionRankSignalMode {
    PureRrf,
    LegacyRankPseudoScore,
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
        conn,
        project,
        current_branch,
        excluded_types,
        "m",
        &mut idx,
        &mut conditions,
        &mut params,
    )?;
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
        conn,
        project,
        current_branch,
        excluded_types,
        "m",
        idx,
        conditions,
        params,
    )?;
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
    let event_time_expr = if sqlite_column_available(conn, "memories", "reference_time_epoch")? {
        "COALESCE(m.reference_time_epoch, m.created_at_epoch)"
    } else {
        "m.created_at_epoch"
    };
    let (temporal_condition, order_epoch) = local_temporal_sql(
        constraint.field,
        has_memory_facts,
        has_memory_fact_invalidations,
        event_time_expr,
    );
    let mut conditions = vec![temporal_condition];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(constraint.start_epoch),
        Box::new(constraint.end_epoch),
    ];
    let mut idx = 3;
    push_context_memory_filters(
        conn,
        project,
        current_branch,
        excluded_types,
        "m",
        &mut idx,
        &mut conditions,
        &mut params,
    )?;
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

fn query_local_fact_channel(
    conn: &Connection,
    project: &str,
    query: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    limit: i64,
) -> Result<Vec<WeightedRankedHit>> {
    let tokens = crate::retrieval::query_expand::core_tokens(query);
    let token_refs = tokens.iter().map(String::as_str).collect::<Vec<_>>();
    let ids = crate::retrieval::temporal::search_fact_memory_ids(
        conn,
        &token_refs,
        Some(project),
        None,
        excluded_types,
        Some(project),
        current_branch,
        limit,
        false,
        crate::retrieval::temporal::FactTimeMode::from_query(query),
    )?;
    let suppressed = crate::memory::suppression::active_suppressed_memory_ids(conn, &ids)?;
    let ids = ids
        .into_iter()
        .filter(|id| !suppressed.contains(id))
        .collect();
    Ok(rank_ordered_hits(ids))
}

fn query_local_vector_channel(
    conn: &Connection,
    project: &str,
    query: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    max_vector_distance: f32,
    allow_remote_embedding: bool,
) -> Result<Vec<WeightedRankedHit>> {
    if !sqlite_table_available(conn, "memory_embeddings")? {
        return Ok(vec![]);
    }
    let query_embedding = if allow_remote_embedding {
        crate::retrieval::embedding::embed_query_if_enabled(query)?
    } else {
        crate::retrieval::embedding::embed_query_local_only_if_enabled(query)?
    };
    let Some(query_embedding) = query_embedding else {
        return Ok(vec![]);
    };
    let profile = query_embedding.profile();
    let mut conditions = vec!["e.model = ?1".to_string(), "e.dimensions = ?2".to_string()];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(profile.model.to_string()),
        Box::new(profile.dimensions as i64),
    ];
    let mut idx = 3;
    push_context_memory_filters(
        conn,
        project,
        current_branch,
        excluded_types,
        "m",
        &mut idx,
        &mut conditions,
        &mut params,
    )?;
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
        hits.push((memory_id, distance));
    }
    hits.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    calibrated_vector_hits(hits, max_vector_distance)
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
        conn,
        project,
        current_branch,
        excluded_types,
        "m",
        &mut idx,
        &mut conditions,
        &mut params,
    )?;
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
    conn: &Connection,
    project: &str,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    alias: &str,
    idx: &mut usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> Result<()> {
    let status_col = qualify(alias, "status");
    let expires_col = qualify(alias, "expires_at_epoch");
    conditions.push(crate::memory::memory_current_filter_sql(
        &status_col,
        &expires_col,
        false,
    ));
    conditions.push(crate::memory::memory_state_key_current_filter_sql(
        table_ref(alias),
    ));
    conditions.push(crate::memory::suppression::memory_policy_filter_sql(
        table_ref(alias),
    ));
    push_owner_included_filter(conn, project, idx, conditions, params)?;
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
    Ok(())
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
    event_time_expr: &str,
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
                         AND {event_time_expr} BETWEEN ?1 AND ?2
                     ))"
                ),
                format!(
                    "COALESCE((
                         SELECT MAX(f.valid_from_epoch)
                         FROM memory_facts f
                         WHERE {fact_event_overlap}
                     ), {event_time_expr})"
                ),
            )
        }
        crate::retrieval::temporal::TemporalField::EventTime => (
            format!("{event_time_expr} BETWEEN ?1 AND ?2"),
            event_time_expr.to_string(),
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

fn sqlite_column_available(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let sql = format!("PRAGMA table_info({})", quote_identifier(table));
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[cfg(test)]
mod tests;
