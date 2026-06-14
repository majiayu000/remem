use anyhow::{bail, Result};
use rusqlite::{params, Connection};

use super::candidates::Cluster;
use super::decisions;
use crate::memory::conflict_common::{common_optional_i64, common_optional_string, common_string};
use crate::memory::edge::{insert_pairwise_conflict_edges, MemoryEdgeWriteContext};
use crate::memory::lifecycle::MemoryLifecycleOp;
use crate::memory::operation::{insert_operation_log, MemoryOperationInput, MemoryOperationPlan};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ConflictOutcome {
    pub operation_id: i64,
    pub edge_count: usize,
}

pub(super) fn record_conflict(
    conn: &mut Connection,
    project: &str,
    cluster: &Cluster,
    conflicting_ids: &[i64],
    reason: Option<&str>,
) -> Result<ConflictOutcome> {
    let tx = conn.transaction()?;
    let metadata = validate_conflicting_ids(&tx, project, cluster, conflicting_ids)?;
    let operation_input = MemoryOperationInput {
        source: "dream".to_string(),
        actor: "dream".to_string(),
        source_project: project.to_string(),
        owner_scope: metadata.owner_scope,
        owner_key: metadata.owner_key,
        memory_type: metadata.memory_type,
        topic_key: metadata.topic_key,
        state_key: metadata.state_key.clone(),
        source_candidate_id: None,
        confidence: None,
    };
    let defer_reason = reason.unwrap_or("dream consolidation deferred unresolved conflict");
    let plan = MemoryOperationPlan::new(
        MemoryLifecycleOp::Conflict,
        metadata.state_key,
        "dream consolidation detected unresolved memory conflict",
    )
    .with_conflicting_ids(metadata.ids.clone())
    .with_defer_reason(defer_reason);
    let operation_id = insert_operation_log(&tx, &operation_input, &plan, None)?;
    let edge_count = insert_pairwise_conflict_edges(
        &tx,
        &metadata.ids,
        MemoryEdgeWriteContext {
            state_key_id: metadata.state_key_id,
            source_candidate_id: None,
            evidence_event_ids: &[],
            source_operation_id: Some(operation_id),
            confidence: None,
            reason: Some(defer_reason),
        },
    )?;
    decisions::record_defer(&tx, project, cluster, reason, operation_id)?;
    tx.commit()?;
    Ok(ConflictOutcome {
        operation_id,
        edge_count,
    })
}

#[derive(Debug)]
struct ConflictMetadata {
    ids: Vec<i64>,
    owner_scope: String,
    owner_key: String,
    memory_type: String,
    topic_key: Option<String>,
    state_key: Option<String>,
    state_key_id: Option<i64>,
}

fn validate_conflicting_ids(
    conn: &Connection,
    project: &str,
    cluster: &Cluster,
    conflicting_ids: &[i64],
) -> Result<ConflictMetadata> {
    let member_ids = cluster
        .members
        .iter()
        .map(|member| member.id)
        .collect::<std::collections::HashSet<_>>();
    let mut ids = conflicting_ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() < 2 {
        bail!("dream conflict requires at least two memory ids");
    }
    if ids.iter().any(|id| !member_ids.contains(id)) {
        bail!("dream conflict ids must be members of the cluster");
    }
    let rows = load_memory_rows(conn, project, &ids)?;
    if rows.len() != ids.len() {
        bail!("dream conflict ids must resolve to active repo memories in the project");
    }
    Ok(ConflictMetadata {
        ids,
        owner_scope: common_string(rows.iter().map(|row| row.owner_scope.as_str()))
            .unwrap_or_else(|| "repo".to_string()),
        owner_key: common_string(rows.iter().map(|row| row.owner_key.as_str()))
            .unwrap_or_else(|| project.to_string()),
        memory_type: common_string(rows.iter().map(|row| row.memory_type.as_str()))
            .or_else(|| {
                cluster
                    .members
                    .first()
                    .map(|member| member.memory_type.clone())
            })
            .unwrap_or_else(|| "memory".to_string()),
        topic_key: common_optional_string(rows.iter().map(|row| row.topic_key.as_deref())),
        state_key: common_optional_string(rows.iter().map(|row| row.state_key.as_deref())),
        state_key_id: common_optional_i64(rows.iter().map(|row| row.state_key_id)),
    })
}

#[derive(Debug)]
struct MemoryRow {
    owner_scope: String,
    owner_key: String,
    memory_type: String,
    topic_key: Option<String>,
    state_key: Option<String>,
    state_key_id: Option<i64>,
}

fn load_memory_rows(conn: &Connection, project: &str, ids: &[i64]) -> Result<Vec<MemoryRow>> {
    let mut rows = Vec::with_capacity(ids.len());
    for id in ids {
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
            params![id, project],
            |row| {
                Ok(MemoryRow {
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
