use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

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
    let decision_cluster = cluster_for_conflicting_ids(cluster, &metadata.ids);
    if let Some(operation_id) = existing_conflict_operation_id(&tx, &metadata.ids)? {
        decisions::record_defer(&tx, project, &decision_cluster, reason, operation_id)?;
        tx.commit()?;
        return Ok(ConflictOutcome {
            operation_id,
            edge_count: 0,
        });
    }
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
    decisions::record_defer(&tx, project, &decision_cluster, reason, operation_id)?;
    tx.commit()?;
    Ok(ConflictOutcome {
        operation_id,
        edge_count,
    })
}

fn cluster_for_conflicting_ids(cluster: &Cluster, ids: &[i64]) -> Cluster {
    let ids = ids
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let mut members = cluster
        .members
        .iter()
        .filter(|member| ids.contains(&member.id))
        .cloned()
        .collect::<Vec<_>>();
    members.sort_by_key(|member| member.id);
    Cluster { members }
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

fn existing_conflict_operation_id(conn: &Connection, ids: &[i64]) -> Result<Option<i64>> {
    let mut operation_id = None;
    for (idx, from_memory_id) in ids.iter().copied().enumerate() {
        for to_memory_id in ids.iter().copied().skip(idx + 1) {
            let pair_operation_id = conn
                .query_row(
                    "SELECT source_operation_id
                     FROM memory_edges
                     WHERE edge_type = 'conflicts'
                       AND (
                            (from_memory_id = ?1 AND to_memory_id = ?2)
                            OR (from_memory_id = ?2 AND to_memory_id = ?1)
                       )
                       AND source_operation_id IS NOT NULL
                     ORDER BY created_at_epoch DESC, id DESC
                     LIMIT 1",
                    params![from_memory_id, to_memory_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            let Some(pair_operation_id) = pair_operation_id else {
                return Ok(None);
            };
            operation_id = Some(operation_id.map_or(pair_operation_id, |current: i64| {
                current.max(pair_operation_id)
            }));
        }
    }
    Ok(operation_id)
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
    let current_filter =
        crate::memory::memory_current_filter_sql("m.status", "m.expires_at_epoch", false);
    for id in ids {
        if let Some(row) = conn
            .query_row(
                &format!(
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
                   AND {current_filter}
                   AND (
                        (m.owner_scope = 'repo' AND m.owner_key = ?2)
                        OR m.target_project = ?2
                        OR (
                            m.owner_scope IS NULL
                            AND m.project = ?2
                            AND COALESCE(m.scope, 'project') != 'global'
                        )
                   )
                 LIMIT 1"
                ),
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
            )
            .optional()?
        {
            rows.push(row);
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dream::candidates::MemoryCandidate;
    use crate::memory::insert_memory;
    use crate::memory::tests_helper::setup_memory_schema;

    fn cluster_for_ids(first_id: i64, second_id: i64) -> Cluster {
        Cluster {
            members: vec![
                MemoryCandidate {
                    id: first_id,
                    topic_key: Some("conflict-a".to_string()),
                    title: "Use provider A".to_string(),
                    content: "Use provider A for embeddings.".to_string(),
                    memory_type: "decision".to_string(),
                    updated_at_epoch: 1,
                },
                MemoryCandidate {
                    id: second_id,
                    topic_key: Some("conflict-b".to_string()),
                    title: "Use provider B".to_string(),
                    content: "Use provider B for embeddings.".to_string(),
                    memory_type: "decision".to_string(),
                    updated_at_epoch: 2,
                },
            ],
        }
    }

    #[test]
    fn record_conflict_rejects_expired_memory() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        setup_memory_schema(&conn);
        let project = "test-dream-expired-conflict";
        let first_id = insert_memory(
            &conn,
            Some("sess-1"),
            project,
            Some("conflict-a"),
            "Use provider A",
            "Use provider A for embeddings.",
            "decision",
            None,
        )?;
        let second_id = insert_memory(
            &conn,
            Some("sess-1"),
            project,
            Some("conflict-b"),
            "Use provider B",
            "Use provider B for embeddings.",
            "decision",
            None,
        )?;
        conn.execute(
            "UPDATE memories SET expires_at_epoch = 1 WHERE id = ?1",
            params![second_id],
        )?;
        let cluster = cluster_for_ids(first_id, second_id);

        let Err(error) = record_conflict(
            &mut conn,
            project,
            &cluster,
            &[first_id, second_id],
            Some("expired provider conflict"),
        ) else {
            panic!("expired conflict memory should be rejected");
        };
        assert!(
            error.to_string().contains("active repo memories"),
            "unexpected error: {error}"
        );
        let conflict_edge_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_edges WHERE edge_type = 'conflicts'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(conflict_edge_count, 0);
        let operation_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_operation_log WHERE operation = 'conflict'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(operation_count, 0);
        Ok(())
    }
}
