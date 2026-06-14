use anyhow::{bail, Result};
use rusqlite::{params, Connection};

use super::{parse_memory_ref_id, ParsedGraphCandidate};
use crate::memory::conflict_common::{common_optional_i64, common_optional_string, common_string};
use crate::memory::edge::{insert_pairwise_conflict_edges, MemoryEdgeWriteContext};
use crate::memory::operation::MemoryOperationInput;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MemoryConflictBridge {
    pub(super) ids: Vec<i64>,
    pub(super) owner_scope: String,
    pub(super) owner_key: String,
    pub(super) memory_type: String,
    pub(super) topic_key: Option<String>,
    pub(super) state_key: Option<String>,
    pub(super) state_key_id: Option<i64>,
}

impl MemoryConflictBridge {
    pub(super) fn operation_input(
        &self,
        source_project: &str,
        candidate_id: i64,
        candidate: &ParsedGraphCandidate,
        actor: &str,
    ) -> MemoryOperationInput {
        MemoryOperationInput {
            source: "graph_candidate".to_string(),
            actor: actor.to_string(),
            source_project: source_project.to_string(),
            owner_scope: self.owner_scope.clone(),
            owner_key: self.owner_key.clone(),
            memory_type: self.memory_type.clone(),
            topic_key: self.topic_key.clone().or(Some(candidate.edge_type.clone())),
            state_key: self.state_key.clone(),
            source_candidate_id: Some(candidate_id),
            confidence: Some(candidate.confidence),
        }
    }

    pub(super) fn insert_memory_edges(
        &self,
        conn: &Connection,
        operation_id: i64,
        candidate: &ParsedGraphCandidate,
    ) -> Result<usize> {
        insert_pairwise_conflict_edges(
            conn,
            &self.ids,
            MemoryEdgeWriteContext {
                state_key_id: self.state_key_id,
                source_candidate_id: None,
                evidence_event_ids: &candidate.evidence_event_ids,
                source_operation_id: Some(operation_id),
                confidence: Some(candidate.confidence),
                reason: Some(candidate.reason.as_str()),
            },
        )
    }
}

pub(super) fn build_memory_conflict_bridge(
    conn: &Connection,
    source_project: &str,
    candidate: &ParsedGraphCandidate,
) -> Result<Option<MemoryConflictBridge>> {
    if candidate.edge_type != "conflicts" {
        return Ok(None);
    }
    let Some(from_id) = parse_memory_ref_id(&candidate.from_ref)? else {
        return Ok(None);
    };
    let Some(to_id) = parse_memory_ref_id(&candidate.to_ref)? else {
        return Ok(None);
    };
    if from_id == to_id {
        bail!("graph conflict candidate cannot point to the same memory twice");
    }
    let mut ids = vec![from_id, to_id];
    ids.sort_unstable();
    ids.dedup();
    let rows = load_memory_ref_rows(conn, source_project, &ids)?;
    if rows.len() != ids.len() {
        bail!("graph conflict candidate references memory outside the active repo scope");
    }
    Ok(Some(MemoryConflictBridge {
        ids,
        owner_scope: common_string(rows.iter().map(|row| row.owner_scope.as_str()))
            .unwrap_or_else(|| "repo".to_string()),
        owner_key: common_string(rows.iter().map(|row| row.owner_key.as_str()))
            .unwrap_or_else(|| source_project.to_string()),
        memory_type: common_string(rows.iter().map(|row| row.memory_type.as_str()))
            .unwrap_or_else(|| "memory".to_string()),
        topic_key: common_optional_string(rows.iter().map(|row| row.topic_key.as_deref())),
        state_key: common_optional_string(rows.iter().map(|row| row.state_key.as_deref())),
        state_key_id: common_optional_i64(rows.iter().map(|row| row.state_key_id)),
    }))
}

#[derive(Debug)]
struct MemoryRefRow {
    owner_scope: String,
    owner_key: String,
    memory_type: String,
    topic_key: Option<String>,
    state_key: Option<String>,
    state_key_id: Option<i64>,
}

fn load_memory_ref_rows(
    conn: &Connection,
    source_project: &str,
    memory_ids: &[i64],
) -> Result<Vec<MemoryRefRow>> {
    let mut rows = Vec::with_capacity(memory_ids.len());
    for memory_id in memory_ids {
        rows.push(conn.query_row(
            "SELECT
                 COALESCE(
                     m.owner_scope,
                     CASE WHEN COALESCE(m.scope, 'project') = 'global' THEN 'user' ELSE 'repo' END
                 ) AS owner_scope,
                 COALESCE(
                     m.owner_key,
                     CASE WHEN COALESCE(m.scope, 'project') = 'global' THEN 'user:default' ELSE m.project END
                 ) AS owner_key,
                 m.memory_type,
                 m.topic_key,
                 sk.state_key,
                 m.state_key_id
             FROM memories m
             LEFT JOIN memory_state_keys sk ON sk.id = m.state_key_id
             WHERE m.id = ?1
               AND m.status = 'active'
               AND (
                    (m.owner_scope = 'repo' AND m.owner_key = ?2)
                    OR m.target_project = ?2
                    OR (
                        m.owner_scope IS NULL
                        AND m.project = ?2
                        AND COALESCE(m.scope, 'project') != 'global'
                    )
               )
             LIMIT 1",
            params![memory_id, source_project],
            |row| {
                Ok(MemoryRefRow {
                    owner_scope: row.get(0)?,
                    owner_key: row.get(1)?,
                    memory_type: row.get(2)?,
                    topic_key: row.get(3)?,
                    state_key: row.get(4)?,
                    state_key_id: row.get(5)?,
                })
            },
        )?);
    }
    Ok(rows)
}
