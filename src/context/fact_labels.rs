use anyhow::Result;
use rusqlite::{types::ToSql, Connection};
use std::collections::HashMap;

use crate::memory::Memory;

use super::format::format_epoch_date;

pub(super) fn annotate_memories_with_temporal_facts(
    conn: &Connection,
    memories: &mut [Memory],
) -> Result<()> {
    if memories.is_empty()
        || !crate::retrieval::temporal::sqlite_table_exists(conn, "memory_facts")?
    {
        return Ok(());
    }
    let ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
    let labels = current_fact_labels_by_memory_id(conn, &ids)?;
    for memory in memories {
        let Some(memory_labels) = labels.get(&memory.id) else {
            continue;
        };
        if memory_labels.is_empty() {
            continue;
        }
        memory.text.push_str("\nTemporal facts: ");
        memory.text.push_str(&memory_labels.join("; "));
    }
    Ok(())
}

fn current_fact_labels_by_memory_id(
    conn: &Connection,
    memory_ids: &[i64],
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
    let mut labels: HashMap<i64, Vec<String>> = HashMap::new();
    for row in rows {
        let (memory_id, subject, predicate, object, valid_from, valid_to) = row?;
        let entry = labels.entry(memory_id).or_default();
        if entry.len() >= 2 {
            continue;
        }
        entry.push(format!(
            "{} {} {} ({})",
            subject,
            predicate,
            object,
            validity_label(valid_from, valid_to)
        ));
    }
    Ok(labels)
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
