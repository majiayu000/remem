use std::collections::HashMap;

use anyhow::Result;
use chrono::{TimeZone, Utc};
use rusqlite::{types::ToSql, Connection};

use crate::memory::Memory;

pub(crate) fn annotate_memories_with_fact_labels(
    conn: &Connection,
    memories: &mut [Memory],
    query: Option<&str>,
    project: Option<&str>,
) -> Result<()> {
    if memories.is_empty() || !super::sqlite_table_exists(conn, "memory_facts")? {
        return Ok(());
    }
    let ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
    let mode = query
        .map(super::FactTimeMode::from_query)
        .unwrap_or(super::FactTimeMode::Current);
    let labels = fact_labels_by_memory_id(conn, &ids, query, project, mode)?;
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

fn fact_labels_by_memory_id(
    conn: &Connection,
    memory_ids: &[i64],
    query: Option<&str>,
    project: Option<&str>,
    mode: super::FactTimeMode,
) -> Result<HashMap<i64, Vec<String>>> {
    let has_invalidated_at_epoch = crate::memory::facts::invalidated_at_epoch_available(conn)?;
    let placeholders = (1..=memory_ids.len())
        .map(|idx| format!("?{idx}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut conditions = vec![format!("f.source_memory_id IN ({placeholders})")];
    let mut params = memory_ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn ToSql>)
        .collect::<Vec<_>>();
    let mut next_idx = memory_ids.len() + 1;
    if let Some(project) = project {
        conditions.push(format!("f.project = ?{next_idx}"));
        params.push(Box::new(project.to_string()));
        next_idx += 1;
    }
    let epoch_idx = next_idx;
    match mode {
        super::FactTimeMode::Current => {
            let now = Utc::now().timestamp();
            conditions.push(crate::memory::facts::current_fact_filter_sql(
                "f",
                has_invalidated_at_epoch,
            ));
            conditions.push(format!(
                "(f.valid_from_epoch IS NULL OR f.valid_from_epoch <= ?{epoch_idx})"
            ));
            conditions.push(format!(
                "(f.valid_to_epoch IS NULL OR f.valid_to_epoch > ?{epoch_idx})"
            ));
            params.push(Box::new(now));
        }
        super::FactTimeMode::AsOf(as_of_epoch) => {
            conditions.push(format!(
                "(f.valid_from_epoch IS NULL OR f.valid_from_epoch <= ?{epoch_idx})"
            ));
            conditions.push(crate::memory::facts::as_of_validity_filter_sql(
                "f",
                epoch_idx,
                has_invalidated_at_epoch,
            ));
            conditions.push(format!("f.learned_at_epoch <= ?{epoch_idx}"));
            if has_invalidated_at_epoch {
                conditions.push(format!(
                    "(f.invalidated_at_epoch IS NULL OR f.invalidated_at_epoch > ?{epoch_idx})"
                ));
            }
            params.push(Box::new(as_of_epoch));
        }
    }
    let sql = format!(
        "SELECT f.source_memory_id, f.subject, f.predicate, f.object,
                f.valid_from_epoch, f.valid_to_epoch
         FROM memory_facts f
         WHERE {}
         ORDER BY f.source_memory_id, COALESCE(f.valid_from_epoch, f.learned_at_epoch) DESC,
                  f.confidence DESC, f.id DESC",
        conditions.join(" AND ")
    );
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
    let match_terms = super::normalized_fact_terms(&query_refs);
    if match_terms.is_empty() {
        return Ok(HashMap::new());
    }
    let mut label_rows: HashMap<i64, Vec<FactLabelRow>> = HashMap::new();
    for (order, row) in rows.enumerate() {
        let (memory_id, subject, predicate, object, valid_from, valid_to) = row?;
        let match_count = fact_match_count(&match_terms, &subject, &predicate, &object);
        if match_count == 0 {
            continue;
        }
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
    Ok(label_rows
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
        .collect())
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
        .map(format_epoch_date_utc)
        .unwrap_or_else(|| "unknown".to_string());
    let to = valid_to
        .map(format_epoch_date_utc)
        .unwrap_or_else(|| "open".to_string());
    format!("valid_from={from}, valid_to={to}")
}

fn format_epoch_date_utc(epoch: i64) -> String {
    Utc.timestamp_opt(epoch, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| epoch.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Context, Result};
    use rusqlite::params;

    fn migrated_conn() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    fn memory(id: i64) -> Memory {
        Memory {
            id,
            session_id: None,
            project: "/repo".to_string(),
            topic_key: None,
            title: format!("Memory {id}"),
            text: "Opaque source body.".to_string(),
            memory_type: "decision".to_string(),
            files: None,
            created_at_epoch: 10,
            updated_at_epoch: 10,
            status: "active".to_string(),
            branch: None,
            scope: "project".to_string(),
        }
    }

    fn insert_memory_row(conn: &Connection, id: i64) -> Result<()> {
        conn.execute(
            "INSERT INTO memories
             (id, session_id, project, topic_key, title, content, memory_type, files,
              created_at_epoch, updated_at_epoch, status, branch, scope)
             VALUES (?1, NULL, '/repo', NULL, ?2, 'Opaque source body.', 'decision',
                     NULL, 10, 10, 'active', NULL, 'project')",
            params![id, format!("Memory {id}")],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_fact(
        conn: &Connection,
        memory_id: i64,
        subject: &str,
        predicate: &str,
        object: &str,
        status: &str,
        valid_from_epoch: Option<i64>,
        valid_to_epoch: Option<i64>,
        learned_at_epoch: i64,
        invalidated_at_epoch: Option<i64>,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO memory_facts
             (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
              learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
              confidence, supersedes_fact_id, status, invalidated_at_epoch,
              created_at_epoch, updated_at_epoch)
             VALUES ('/repo', ?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, '[]',
                     0.95, NULL, ?8, ?9, ?6, ?6)",
            params![
                subject,
                predicate,
                object,
                valid_from_epoch,
                valid_to_epoch,
                learned_at_epoch,
                memory_id,
                status,
                invalidated_at_epoch
            ],
        )?;
        Ok(())
    }

    #[test]
    fn format_epoch_date_utc_uses_utc_day() {
        assert_eq!(format_epoch_date_utc(1_767_308_400), "2026-01-01");
    }

    #[test]
    fn as_of_labels_render_historical_snapshot() -> Result<()> {
        let conn = migrated_conn()?;
        insert_memory_row(&conn, 1)?;
        let as_of = chrono::NaiveDate::from_ymd_opt(2026, 1, 15)
            .and_then(|date| date.and_hms_opt(0, 0, 0))
            .context("valid as-of date")?
            .and_utc()
            .timestamp();
        insert_fact(
            &conn,
            1,
            "HarborMint owner",
            "verified_by",
            "Ada Old",
            "stale",
            Some(as_of - 10_000),
            Some(as_of + 1_000),
            as_of - 900,
            Some(as_of + 500),
        )?;
        insert_fact(
            &conn,
            1,
            "HarborMint owner",
            "verified_by",
            "Nia New",
            "active",
            Some(as_of + 100),
            None,
            as_of + 100,
            None,
        )?;
        let mut memories = vec![memory(1)];

        annotate_memories_with_fact_labels(
            &conn,
            &mut memories,
            Some("who owned HarborMint as of 2026-01-15"),
            Some("/repo"),
        )?;

        assert!(memories[0]
            .text
            .contains("HarborMint owner verified_by Ada Old"));
        assert!(!memories[0].text.contains("Nia New"));
        Ok(())
    }

    #[test]
    fn labels_skip_facts_that_do_not_match_query_terms() -> Result<()> {
        let conn = migrated_conn()?;
        insert_memory_row(&conn, 1)?;
        let now = Utc::now().timestamp();
        insert_fact(
            &conn,
            1,
            "HarborMint owner",
            "verified_by",
            "Ada Lovelace",
            "active",
            Some(now - 1_000),
            None,
            now - 900,
            None,
        )?;
        insert_fact(
            &conn,
            1,
            "UnrelatedService",
            "blocked_by",
            "North Region",
            "active",
            Some(now - 500),
            None,
            now - 400,
            None,
        )?;
        let mut memories = vec![memory(1)];

        annotate_memories_with_fact_labels(
            &conn,
            &mut memories,
            Some("who owns HarborMint"),
            Some("/repo"),
        )?;

        assert!(memories[0]
            .text
            .contains("HarborMint owner verified_by Ada Lovelace"));
        assert!(!memories[0].text.contains("UnrelatedService"));
        Ok(())
    }

    #[test]
    fn labels_filter_facts_by_requested_project() -> Result<()> {
        let conn = migrated_conn()?;
        insert_memory_row(&conn, 1)?;
        let now = Utc::now().timestamp();
        insert_fact(
            &conn,
            1,
            "HarborMint",
            "verified_by",
            "Other Project",
            "active",
            Some(now - 1_000),
            None,
            now - 800,
            None,
        )?;
        conn.execute(
            "UPDATE memory_facts SET project = '/other' WHERE source_memory_id = 1",
            [],
        )?;
        insert_fact(
            &conn,
            1,
            "HarborMint",
            "verified_by",
            "Toma Reed",
            "active",
            Some(now - 1_000),
            None,
            now - 900,
            None,
        )?;
        let mut memories = vec![memory(1)];

        annotate_memories_with_fact_labels(
            &conn,
            &mut memories,
            Some("who verified HarborMint with Toma Reed"),
            Some("/repo"),
        )?;

        assert!(memories[0].text.contains("Toma Reed"));
        assert!(!memories[0].text.contains("Other Project"));
        Ok(())
    }
}
