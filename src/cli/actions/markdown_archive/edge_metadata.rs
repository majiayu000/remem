use super::{column_exists, ImportedMarkdownMemory, MarkdownMemoryEdgeMetadata};
use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};

const EDGE_TYPES: &[&str] = &[
    "supersedes",
    "duplicates",
    "conflicts",
    "derived_from",
    "merged_into",
    "split_from",
];

pub(super) fn load_markdown_memory_edges(
    conn: &Connection,
    memory_id: i64,
) -> Result<Vec<MarkdownMemoryEdgeMetadata>> {
    if !column_exists(conn, "memory_edges", "edge_type")? {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT e.id, e.edge_type, e.from_memory_id, e.to_memory_id,
                sk.owner_scope, sk.owner_key, sk.memory_type, sk.state_key,
                e.evidence_event_ids, e.source_candidate_id, e.source_operation_id,
                e.confidence, e.reason, e.created_at_epoch
         FROM memory_edges e
         LEFT JOIN memory_state_keys sk ON sk.id = e.state_key_id
         WHERE e.from_memory_id = ?1
         ORDER BY e.created_at_epoch, e.id",
    )?;
    let rows = stmt.query_map(params![memory_id], |row| {
        let evidence_json: Option<String> = row.get(8)?;
        Ok(MarkdownMemoryEdgeMetadata {
            source_edge_id: row.get(0)?,
            edge_type: row.get(1)?,
            from_source_id: row.get(2)?,
            to_source_id: row.get(3)?,
            state_owner_scope: row.get(4)?,
            state_owner_key: row.get(5)?,
            state_memory_type: row.get(6)?,
            state_key: row.get(7)?,
            evidence_event_ids: parse_edge_event_ids(evidence_json, 8)?,
            source_candidate_id: row.get(9)?,
            source_operation_id: row.get(10)?,
            confidence: row.get(11)?,
            reason: row.get(12)?,
            created_at_epoch: row.get(13)?,
        })
    })?;
    crate::db::query::collect_rows(rows)
}

pub(super) fn replace_markdown_memory_edges(
    conn: &Connection,
    imported: &[ImportedMarkdownMemory],
) -> Result<()> {
    let source_to_target: HashMap<i64, i64> = imported
        .iter()
        .filter_map(|memory| {
            memory
                .doc
                .metadata
                .source_id
                .map(|source_id| (source_id, memory.memory_id))
        })
        .collect();
    let has_edge_metadata = imported
        .iter()
        .any(|memory| memory.doc.metadata.edges.is_some());
    if !has_edge_metadata {
        return Ok(());
    }
    let has_nonempty_edges = imported.iter().any(|memory| {
        memory
            .doc
            .metadata
            .edges
            .as_ref()
            .is_some_and(|edges| !edges.is_empty())
    });
    if !column_exists(conn, "memory_edges", "edge_type")? {
        if !has_nonempty_edges {
            return Ok(());
        }
        anyhow::bail!(
            "markdown archive contains memory_edges but target database lacks memory_edges table"
        );
    }

    conn.execute_batch("SAVEPOINT remem_restore_markdown_edges")?;
    let result = (|| -> Result<()> {
        let mut replaced_from_ids = HashSet::new();
        for memory in imported
            .iter()
            .filter(|memory| memory.doc.metadata.edges.is_some())
        {
            if replaced_from_ids.insert(memory.memory_id) {
                conn.execute(
                    "DELETE FROM memory_edges WHERE from_memory_id = ?1",
                    params![memory.memory_id],
                )?;
            }
        }

        let mut inserted = HashSet::new();
        for memory in imported {
            let Some(edges) = memory.doc.metadata.edges.as_deref() else {
                continue;
            };
            for edge in edges {
                let Some(from_memory_id) =
                    remap_source_memory(edge.from_source_id, memory, &source_to_target)
                else {
                    continue;
                };
                let to_memory_id = match edge.to_source_id {
                    Some(source_id) => source_to_target.get(&source_id).copied(),
                    None => None,
                };
                if edge.to_source_id.is_some() && to_memory_id.is_none() {
                    continue;
                }
                validate_edge_type(&edge.edge_type)?;
                let state_key_id =
                    remapped_edge_state_key_id(conn, edge, Some(from_memory_id), to_memory_id)?;
                let key = (
                    edge.edge_type.as_str(),
                    Some(from_memory_id),
                    to_memory_id,
                    state_key_id,
                    edge.reason.as_deref(),
                    edge.created_at_epoch,
                );
                if inserted.insert(key) {
                    insert_markdown_memory_edge(
                        conn,
                        edge,
                        Some(from_memory_id),
                        to_memory_id,
                        state_key_id,
                    )?;
                }
            }
        }
        Ok(())
    })();

    match result {
        Ok(()) => {
            conn.execute_batch("RELEASE SAVEPOINT remem_restore_markdown_edges")?;
            Ok(())
        }
        Err(error) => {
            if let Err(rollback_error) = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT remem_restore_markdown_edges; RELEASE SAVEPOINT remem_restore_markdown_edges",
            ) {
                return Err(rollback_error)
                    .context(format!("rollback markdown edge restore after failure: {error}"));
            }
            Err(error)
        }
    }
}

fn remap_source_memory(
    source_id: Option<i64>,
    memory: &ImportedMarkdownMemory,
    source_to_target: &HashMap<i64, i64>,
) -> Option<i64> {
    match source_id {
        Some(source_id) => source_to_target.get(&source_id).copied(),
        None => Some(memory.memory_id),
    }
}

fn insert_markdown_memory_edge(
    conn: &Connection,
    edge: &MarkdownMemoryEdgeMetadata,
    from_memory_id: Option<i64>,
    to_memory_id: Option<i64>,
    state_key_id: Option<i64>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, source_candidate_id,
          evidence_event_ids, source_operation_id, confidence, reason, created_at_epoch)
         VALUES (?1, ?2, ?3, ?4, NULL, NULL, NULL, ?5, ?6, ?7)",
        params![
            edge.edge_type,
            from_memory_id,
            to_memory_id,
            state_key_id,
            edge.confidence,
            edge.reason,
            edge.created_at_epoch,
        ],
    )
    .context("insert markdown memory edge")?;
    Ok(())
}

fn remapped_edge_state_key_id(
    conn: &Connection,
    edge: &MarkdownMemoryEdgeMetadata,
    from_memory_id: Option<i64>,
    to_memory_id: Option<i64>,
) -> Result<Option<i64>> {
    if let Some(memory_id) = to_memory_id {
        if let Some(state_key_id) = memory_row_state_key_id(conn, memory_id)? {
            return Ok(Some(state_key_id));
        }
    }
    if let Some(memory_id) = from_memory_id {
        if let Some(state_key_id) = memory_row_state_key_id(conn, memory_id)? {
            return Ok(Some(state_key_id));
        }
    }
    let (Some(owner_scope), Some(owner_key), Some(memory_type), Some(state_key)) = (
        edge.state_owner_scope.as_deref(),
        edge.state_owner_key.as_deref(),
        edge.state_memory_type.as_deref(),
        edge.state_key.as_deref(),
    ) else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT id FROM memory_state_keys
         WHERE owner_scope = ?1
           AND owner_key = ?2
           AND memory_type = ?3
           AND state_key = ?4",
        params![owner_scope, owner_key, memory_type, state_key],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn memory_row_state_key_id(conn: &Connection, memory_id: i64) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT state_key_id FROM memories WHERE id = ?1",
        params![memory_id],
        |row| row.get(0),
    )
    .optional()
    .map(|value| value.flatten())
    .map_err(Into::into)
}

fn validate_edge_type(edge_type: &str) -> Result<()> {
    if EDGE_TYPES.contains(&edge_type) {
        Ok(())
    } else {
        Err(anyhow!("unsupported markdown memory edge_type {edge_type}"))
    }
}

fn parse_edge_event_ids(json: Option<String>, column: usize) -> rusqlite::Result<Vec<i64>> {
    match json {
        Some(json) => serde_json::from_str::<Vec<i64>>(&json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                column,
                rusqlite::types::Type::Text,
                Box::new(err),
            )
        }),
        None => Ok(Vec::new()),
    }
}
