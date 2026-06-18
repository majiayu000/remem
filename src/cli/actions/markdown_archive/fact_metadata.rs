use super::{column_exists, MarkdownMemoryDocument, MarkdownMemoryFactMetadata};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use std::collections::BTreeMap;

pub(super) fn load_markdown_memory_facts(
    conn: &Connection,
    memory_id: i64,
) -> Result<Vec<MarkdownMemoryFactMetadata>> {
    if !column_exists(conn, "memory_facts", "source_memory_id")? {
        return Ok(Vec::new());
    }
    let invalidated_expr = if column_exists(conn, "memory_facts", "invalidated_at_epoch")? {
        "invalidated_at_epoch"
    } else {
        "NULL AS invalidated_at_epoch"
    };
    let sql = format!(
        "SELECT id, project, subject, predicate, object, valid_from_epoch,
                valid_to_epoch, learned_at_epoch, source_observation_id,
                source_event_ids, confidence, supersedes_fact_id, status,
                created_at_epoch, updated_at_epoch, {invalidated_expr}
         FROM memory_facts
         WHERE source_memory_id = ?1
         ORDER BY id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([memory_id], |row| {
        let source_event_json: String = row.get(9)?;
        let source_event_ids =
            serde_json::from_str::<Vec<i64>>(&source_event_json).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(
                    9,
                    rusqlite::types::Type::Text,
                    Box::new(err),
                )
            })?;
        Ok(MarkdownMemoryFactMetadata {
            source_id: row.get(0)?,
            project: row.get(1)?,
            subject: row.get(2)?,
            predicate: row.get(3)?,
            object: row.get(4)?,
            valid_from_epoch: row.get(5)?,
            valid_to_epoch: row.get(6)?,
            learned_at_epoch: row.get(7)?,
            source_observation_id: row.get(8)?,
            source_event_ids,
            confidence: row.get(10)?,
            supersedes_fact_id: row.get(11)?,
            status: row.get(12)?,
            created_at_epoch: row.get(13)?,
            updated_at_epoch: row.get(14)?,
            invalidated_at_epoch: row.get(15)?,
        })
    })?;
    crate::db::query::collect_rows(rows)
        .with_context(|| format!("load markdown memory facts for memory id={memory_id}"))
}

pub(super) fn replace_markdown_memory_facts(
    conn: &Connection,
    memory_id: i64,
    doc: &MarkdownMemoryDocument,
) -> Result<()> {
    let Some(facts) = doc.metadata.facts.as_ref() else {
        return Ok(());
    };
    if !column_exists(conn, "memory_facts", "source_memory_id")? {
        if facts.is_empty() {
            return Ok(());
        }
        anyhow::bail!(
            "markdown archive contains memory_facts but target database lacks memory_facts table"
        );
    }
    conn.execute(
        "DELETE FROM memory_facts WHERE source_memory_id = ?1",
        [memory_id],
    )?;
    let has_invalidated_at_epoch = column_exists(conn, "memory_facts", "invalidated_at_epoch")?;
    let mut remapped_ids = BTreeMap::new();
    for fact in facts {
        let new_id = insert_markdown_fact(
            conn,
            memory_id,
            fact,
            has_invalidated_at_epoch,
            source_observation_id(conn, fact.source_observation_id)?,
        )?;
        if let Some(source_id) = fact.source_id {
            remapped_ids.insert(source_id, new_id);
        }
    }
    for fact in facts {
        let Some(source_id) = fact.source_id else {
            continue;
        };
        let Some(new_id) = remapped_ids.get(&source_id).copied() else {
            continue;
        };
        let supersedes_fact_id = fact
            .supersedes_fact_id
            .and_then(|old_id| remapped_ids.get(&old_id).copied());
        conn.execute(
            "UPDATE memory_facts SET supersedes_fact_id = ?1 WHERE id = ?2",
            rusqlite::params![supersedes_fact_id, new_id],
        )?;
    }
    Ok(())
}

fn insert_markdown_fact(
    conn: &Connection,
    memory_id: i64,
    fact: &MarkdownMemoryFactMetadata,
    has_invalidated_at_epoch: bool,
    source_observation_id: Option<i64>,
) -> Result<i64> {
    validate_fact(fact)?;
    let source_event_ids = serde_json::to_string(&fact.source_event_ids)?;
    if has_invalidated_at_epoch {
        conn.execute(
            "INSERT INTO memory_facts
             (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
              learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
              confidence, supersedes_fact_id, status, created_at_epoch, updated_at_epoch,
              invalidated_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12, ?13, ?14, ?15)",
            rusqlite::params![
                fact.project,
                fact.subject,
                fact.predicate,
                fact.object,
                fact.valid_from_epoch,
                fact.valid_to_epoch,
                fact.learned_at_epoch,
                memory_id,
                source_observation_id,
                source_event_ids,
                fact.confidence,
                fact.status,
                fact.created_at_epoch,
                fact.updated_at_epoch,
                fact.invalidated_at_epoch,
            ],
        )?;
    } else {
        conn.execute(
            "INSERT INTO memory_facts
             (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
              learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
              confidence, supersedes_fact_id, status, created_at_epoch, updated_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12, ?13, ?14)",
            rusqlite::params![
                fact.project,
                fact.subject,
                fact.predicate,
                fact.object,
                fact.valid_from_epoch,
                fact.valid_to_epoch,
                fact.learned_at_epoch,
                memory_id,
                source_observation_id,
                source_event_ids,
                fact.confidence,
                fact.status,
                fact.created_at_epoch,
                fact.updated_at_epoch,
            ],
        )?;
    }
    Ok(conn.last_insert_rowid())
}

fn validate_fact(fact: &MarkdownMemoryFactMetadata) -> Result<()> {
    if fact.project.trim().is_empty() {
        anyhow::bail!("markdown memory fact project must not be empty");
    }
    if fact.subject.trim().is_empty() {
        anyhow::bail!("markdown memory fact subject must not be empty");
    }
    if fact.object.trim().is_empty() {
        anyhow::bail!("markdown memory fact object must not be empty");
    }
    if !is_supported_fact_predicate(&fact.predicate) {
        anyhow::bail!(
            "unsupported markdown memory fact predicate {}",
            fact.predicate
        );
    }
    if !matches!(fact.status.as_str(), "active" | "stale") {
        anyhow::bail!("unsupported markdown memory fact status {}", fact.status);
    }
    if !(0.0..=1.0).contains(&fact.confidence) {
        anyhow::bail!("markdown memory fact confidence out of range");
    }
    if let (Some(valid_from), Some(valid_to)) = (fact.valid_from_epoch, fact.valid_to_epoch) {
        if valid_to < valid_from {
            anyhow::bail!("markdown memory fact valid_to_epoch cannot be before valid_from_epoch");
        }
    }
    Ok(())
}

fn is_supported_fact_predicate(value: &str) -> bool {
    matches!(
        value,
        "fixed_by"
            | "verified_by"
            | "supersedes"
            | "blocked_by"
            | "uses_file"
            | "uses_command"
            | "affects_project"
    )
}

fn source_observation_id(
    conn: &Connection,
    source_observation_id: Option<i64>,
) -> Result<Option<i64>> {
    let Some(id) = source_observation_id else {
        return Ok(None);
    };
    if !column_exists(conn, "observations", "id")? {
        return Ok(None);
    }
    let exists = conn
        .query_row("SELECT id FROM observations WHERE id = ?1", [id], |row| {
            row.get::<_, i64>(0)
        })
        .optional()?
        .is_some();
    Ok(exists.then_some(id))
}
