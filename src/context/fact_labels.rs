use anyhow::Result;
use rusqlite::{types::ToSql, Connection};
use std::collections::HashMap;

use crate::memory::Memory;

use super::format::format_epoch_date;

pub(super) fn annotate_memories_with_temporal_facts_for_query(
    conn: &Connection,
    memories: &mut [Memory],
    query: Option<&str>,
) -> Result<()> {
    if memories.is_empty()
        || !crate::retrieval::temporal::sqlite_table_exists(conn, "memory_facts")?
    {
        return Ok(());
    }
    let ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
    let labels = current_fact_labels_by_memory_id(conn, &ids, query)?;
    for memory in memories {
        let Some(memory_labels) = labels.get(&memory.id) else {
            continue;
        };
        if memory_labels.is_empty() {
            continue;
        }
        memory.text = format!(
            "Temporal facts: {}\n{}",
            memory_labels.join("; "),
            memory.text
        );
    }
    Ok(())
}

fn current_fact_labels_by_memory_id(
    conn: &Connection,
    memory_ids: &[i64],
    query: Option<&str>,
) -> Result<HashMap<i64, Vec<String>>> {
    let has_invalidated_at_epoch = crate::memory::facts::invalidated_at_epoch_available(conn)?;
    let placeholders = (1..=memory_ids.len())
        .map(|idx| format!("?{idx}"))
        .collect::<Vec<_>>()
        .join(", ");
    let now_idx = memory_ids.len() + 1;
    let current_filter =
        crate::memory::facts::current_fact_filter_sql("f", has_invalidated_at_epoch);
    let sql = format!(
        "SELECT f.source_memory_id, f.subject, f.predicate, f.object,
                f.valid_from_epoch, f.valid_to_epoch
         FROM memory_facts f
         WHERE f.source_memory_id IN ({placeholders})
           AND {current_filter}
           AND (f.valid_from_epoch IS NULL OR f.valid_from_epoch <= ?{now_idx})
           AND (f.valid_to_epoch IS NULL OR f.valid_to_epoch > ?{now_idx})
         ORDER BY f.source_memory_id, COALESCE(f.valid_from_epoch, f.learned_at_epoch) DESC,
                  f.confidence DESC, f.id DESC"
    );
    let now = chrono::Utc::now().timestamp();
    let mut params = memory_ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn ToSql>)
        .collect::<Vec<_>>();
    params.push(Box::new(now));
    let refs = crate::db::to_sql_refs(&params);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, Option<i64>>(5)?,
        ))
    })?;
    let query_tokens = query
        .map(crate::retrieval::query_expand::core_tokens)
        .unwrap_or_default();
    let query_refs = query_tokens.iter().map(String::as_str).collect::<Vec<_>>();
    let match_terms = crate::retrieval::temporal::normalized_fact_terms(&query_refs);
    let mut label_rows: HashMap<i64, Vec<FactLabelRow>> = HashMap::new();
    for (order, row) in rows.enumerate() {
        let (memory_id, subject, predicate, object, valid_from, valid_to) = row?;
        let match_count = fact_match_count(&match_terms, &subject, &predicate, &object);
        label_rows.entry(memory_id).or_default().push(FactLabelRow {
            label: format!(
                "{} {} {} ({})",
                subject,
                predicate,
                object,
                validity_label(valid_from, valid_to)
            ),
            match_count,
            order,
        });
    }
    let labels = label_rows
        .into_iter()
        .map(|(memory_id, mut rows)| {
            rows.sort_by(|left, right| {
                right
                    .match_count
                    .cmp(&left.match_count)
                    .then_with(|| left.order.cmp(&right.order))
            });
            (
                memory_id,
                rows.into_iter()
                    .take(2)
                    .map(|row| row.label)
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    Ok(labels)
}

struct FactLabelRow {
    label: String,
    match_count: usize,
    order: usize,
}

fn fact_match_count(terms: &[String], subject: &str, predicate: &str, object: &str) -> usize {
    if terms.is_empty() {
        return 0;
    }
    let haystack = format!("{subject} {predicate} {object}").to_lowercase();
    terms
        .iter()
        .filter(|term| haystack.contains(term.as_str()))
        .count()
}

fn validity_label(valid_from: Option<i64>, valid_to: Option<i64>) -> String {
    let from = valid_from
        .map(format_epoch_date)
        .unwrap_or_else(|| "unknown".to_string());
    let to = valid_to
        .map(format_epoch_date)
        .unwrap_or_else(|| "open".to_string());
    format!("valid_from={from}, valid_to={to}")
}
