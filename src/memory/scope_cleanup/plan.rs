use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

use crate::memory::lifecycle::MemoryLifecycleOp;
use crate::memory::operation::{insert_operation_log, MemoryOperationInput, MemoryOperationPlan};

use super::audit::load_memory_audit_rows;
use super::mutate::{insert_scope_cleanup_event, load_target, ObjectMutation};
use super::preference_cluster::preference_clusters;
use super::ObjectRef;

pub const CLEANUP_PLANNER_VERSION: &str = "memory-cleanup-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryCleanupPlan {
    pub project: String,
    pub created_at_epoch: i64,
    pub planner_version: String,
    pub groups: Vec<MemoryCleanupGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryCleanupGroup {
    pub cluster_key: String,
    pub owner_scope: Option<String>,
    pub owner_key: Option<String>,
    pub memory_type: String,
    pub state_key: Option<String>,
    pub current_id: i64,
    pub stale_ids: Vec<i64>,
    pub reason: String,
    pub confidence: f64,
    pub preview: Vec<String>,
    pub merged_content: Option<String>,
    pub row_snapshots: Vec<MemoryCleanupRowSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryCleanupRowSnapshot {
    pub id: i64,
    pub project: String,
    pub scope: Option<String>,
    pub source_project: Option<String>,
    pub target_project: Option<String>,
    pub status: String,
    pub content_sha256: String,
    pub updated_at_epoch: i64,
    pub owner_scope: Option<String>,
    pub owner_key: Option<String>,
    pub memory_type: String,
    pub topic_key: Option<String>,
    pub state_key_id: Option<i64>,
    pub state_key: Option<String>,
    pub current_memory_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryCleanupApplyResult {
    pub project: String,
    pub planner_version: String,
    pub groups_applied: usize,
    pub current_ids: Vec<i64>,
    pub stale_ids: Vec<i64>,
    pub operation_ids: Vec<i64>,
    pub edge_count: usize,
    pub affected: Vec<ObjectMutation>,
}

pub fn build_preference_cleanup_plan(
    conn: &Connection,
    project: &str,
) -> Result<MemoryCleanupPlan> {
    let memories = load_memory_audit_rows(conn, project)?;
    let clusters = preference_clusters(&memories, project);
    let mut groups = Vec::with_capacity(clusters.len());

    for cluster in clusters {
        let current_ref = ObjectRef::parse(&cluster.canonical_ref)?;
        let stale_ids = cluster
            .refs
            .iter()
            .filter(|object_ref| *object_ref != &cluster.canonical_ref)
            .map(|object_ref| ObjectRef::parse(object_ref).map(|parsed| parsed.id))
            .collect::<Result<Vec<_>>>()?;
        if stale_ids.is_empty() {
            continue;
        }
        let mut ids = Vec::with_capacity(stale_ids.len() + 1);
        ids.push(current_ref.id);
        ids.extend(stale_ids.iter().copied());
        let row_snapshots = load_row_snapshots(conn, &ids)?;
        let current = snapshot_for(&row_snapshots, current_ref.id)?;
        let preview = row_snapshots
            .iter()
            .take(4)
            .map(|row| format!("memory:{} {}", row.id, row.status))
            .collect();
        groups.push(MemoryCleanupGroup {
            cluster_key: cluster.cluster_key,
            owner_scope: current.owner_scope.clone(),
            owner_key: current.owner_key.clone(),
            memory_type: current.memory_type.clone(),
            state_key: current.state_key.clone(),
            current_id: current_ref.id,
            stale_ids,
            reason: cluster.reason,
            confidence: 1.0,
            preview,
            merged_content: cluster.merged_content,
            row_snapshots,
        });
    }

    Ok(MemoryCleanupPlan {
        project: project.to_string(),
        created_at_epoch: chrono::Utc::now().timestamp(),
        planner_version: CLEANUP_PLANNER_VERSION.to_string(),
        groups,
    })
}

pub fn apply_memory_cleanup_plan(
    conn: &Connection,
    plan: &MemoryCleanupPlan,
) -> Result<MemoryCleanupApplyResult> {
    if plan.planner_version != CLEANUP_PLANNER_VERSION {
        bail!(
            "unsupported cleanup planner version: {}",
            plan.planner_version
        );
    }

    let tx = conn.unchecked_transaction()?;
    validate_plan_shape(plan)?;
    for group in &plan.groups {
        validate_group(&tx, plan, group)?;
    }

    let now = chrono::Utc::now().timestamp();
    let mut affected = Vec::new();
    let mut current_ids = Vec::new();
    let mut stale_ids = Vec::new();
    let mut operation_ids = Vec::new();
    let mut edge_count = 0usize;

    for group in &plan.groups {
        let current_ref = ObjectRef::memory(group.current_id);
        let canonical = load_target(&tx, current_ref)?;
        let current_snapshot = snapshot_for(&group.row_snapshots, group.current_id)?;
        let merged = group.merged_content.as_deref();
        let final_text = if let Some(merged) = merged {
            merged.to_string()
        } else {
            tx.query_row(
                "SELECT content FROM memories WHERE id = ?1",
                [group.current_id],
                |row| row.get::<_, String>(0),
            )?
        };
        let affected_ids = std::iter::once(group.current_id)
            .chain(group.stale_ids.iter().copied())
            .collect::<Vec<_>>();
        crate::memory::preference::compilation::enqueue_for_memory_ids(&tx, &affected_ids)?;
        crate::memory::preference::reinforcement::reconcile_cleanup_preference(
            &tx,
            group.current_id,
            &group.stale_ids,
            &final_text,
            now,
        )?;
        let updated = tx.execute(
            "UPDATE memories
             SET content = COALESCE(?1, content),
                 status = 'active',
                 updated_at_epoch = ?2
             WHERE id = ?3",
            params![merged, now, group.current_id],
        )?;
        if updated != 1 {
            bail!(
                "failed to update cleanup current memory {}",
                group.current_id
            );
        }
        current_ids.push(group.current_id);
        affected.push(ObjectMutation {
            object_ref: current_ref.to_string(),
            title: canonical.title.clone(),
            previous_status: canonical.status.clone(),
            new_status: "active".to_string(),
            previous_owner: canonical.owner.clone(),
            new_owner: canonical.owner.clone(),
        });
        insert_scope_cleanup_event(
            &tx,
            "memory-cleanup",
            &canonical,
            "active",
            &canonical.owner,
            Some(group.reason.as_str()),
            now,
        )?;

        if let Some(state_key_id) = current_snapshot.state_key_id {
            tx.execute(
                "UPDATE memory_state_keys
                 SET current_memory_id = ?1, updated_at_epoch = ?2
                 WHERE id = ?3",
                params![group.current_id, now, state_key_id],
            )?;
        }

        for stale_id in &group.stale_ids {
            let stale_ref = ObjectRef::memory(*stale_id);
            let target = load_target(&tx, stale_ref)?;
            let updated = tx.execute(
                "UPDATE memories SET status = 'stale', updated_at_epoch = ?1 WHERE id = ?2",
                params![now, stale_id],
            )?;
            if updated != 1 {
                bail!("failed to stale cleanup memory {stale_id}");
            }
            stale_ids.push(*stale_id);
            affected.push(ObjectMutation {
                object_ref: stale_ref.to_string(),
                title: target.title.clone(),
                previous_status: target.status.clone(),
                new_status: "stale".to_string(),
                previous_owner: target.owner.clone(),
                new_owner: target.owner.clone(),
            });
            insert_scope_cleanup_event(
                &tx,
                "memory-cleanup",
                &target,
                "stale",
                &target.owner,
                Some("duplicate preference superseded by cleanup plan"),
                now,
            )?;
        }

        let operation_id = insert_cleanup_operation_log(&tx, plan, group)?;
        operation_ids.push(operation_id);
        edge_count += crate::memory::edge::insert_replacement_edges(
            &tx,
            crate::memory::edge::MemoryEdgeType::Duplicates,
            &group.stale_ids,
            group.current_id,
            crate::memory::edge::MemoryEdgeWriteContext {
                state_key_id: current_snapshot.state_key_id,
                source_operation_id: Some(operation_id),
                confidence: Some(group.confidence),
                reason: Some(group.reason.as_str()),
                ..Default::default()
            },
        )?;
    }

    tx.commit()?;
    Ok(MemoryCleanupApplyResult {
        project: plan.project.clone(),
        planner_version: plan.planner_version.clone(),
        groups_applied: plan.groups.len(),
        current_ids,
        stale_ids,
        operation_ids,
        edge_count,
        affected,
    })
}

fn validate_plan_shape(plan: &MemoryCleanupPlan) -> Result<()> {
    let mut ids = HashSet::new();
    for group in &plan.groups {
        for id in std::iter::once(group.current_id).chain(group.stale_ids.iter().copied()) {
            if !ids.insert(id) {
                bail!("cleanup plan lists memory:{id} in more than one action");
            }
        }
    }
    Ok(())
}

fn validate_group(
    conn: &Connection,
    plan: &MemoryCleanupPlan,
    group: &MemoryCleanupGroup,
) -> Result<()> {
    if group.stale_ids.contains(&group.current_id) {
        bail!(
            "cleanup group {} lists current id {} as stale",
            group.cluster_key,
            group.current_id
        );
    }
    if group.memory_type != "preference" {
        bail!(
            "unsupported cleanup group memory type {}",
            group.memory_type
        );
    }
    let mut expected_ids = group.stale_ids.clone();
    expected_ids.push(group.current_id);
    expected_ids.sort_unstable();
    expected_ids.dedup();
    let mut snapshot_ids = group
        .row_snapshots
        .iter()
        .map(|snapshot| snapshot.id)
        .collect::<Vec<_>>();
    snapshot_ids.sort_unstable();
    snapshot_ids.dedup();
    if snapshot_ids != expected_ids {
        bail!(
            "cleanup group {} row snapshots do not match current/stale ids",
            group.cluster_key
        );
    }

    let current_snapshot = snapshot_for(&group.row_snapshots, group.current_id)?;
    if group.owner_scope != current_snapshot.owner_scope
        || group.owner_key != current_snapshot.owner_key
    {
        bail!(
            "cleanup group {} owner does not match current row owner",
            group.cluster_key
        );
    }
    if group.state_key != current_snapshot.state_key {
        bail!(
            "cleanup group {} state key does not match current row",
            group.cluster_key
        );
    }
    let current_owner = current_snapshot.owner_namespace(&plan.project);
    let current_state_key_id = current_snapshot.state_key_id;
    let current_state_key = current_snapshot.state_key.as_deref();
    let topic_group = group.cluster_key.starts_with("topic:");

    for snapshot in &group.row_snapshots {
        let current = load_row_snapshot(conn, snapshot.id)?
            .ok_or_else(|| anyhow!("cleanup plan row {} no longer exists", snapshot.id))?;
        if &current != snapshot {
            bail!(
                "cleanup plan row {} changed since dry-run; refresh the plan before applying",
                snapshot.id
            );
        }
        if snapshot.status != "active" {
            bail!("cleanup plan row {} is no longer active", snapshot.id);
        }
        if snapshot.memory_type != group.memory_type {
            bail!(
                "cleanup plan row {} type {} does not match group type {}",
                snapshot.id,
                snapshot.memory_type,
                group.memory_type
            );
        }
        if !snapshot.belongs_to_project(&plan.project) {
            bail!(
                "cleanup plan row {} does not belong to project {}",
                snapshot.id,
                plan.project
            );
        }
        if snapshot.owner_namespace(&plan.project) != current_owner {
            bail!(
                "cleanup plan row {} owner does not match current row owner",
                snapshot.id
            );
        }
        match (current_state_key_id, current_state_key) {
            (Some(state_key_id), _) if snapshot.state_key_id != Some(state_key_id) => {
                bail!(
                    "cleanup plan row {} state key does not match current row",
                    snapshot.id
                );
            }
            (None, Some(state_key)) if snapshot.state_key.as_deref() != Some(state_key) => {
                bail!(
                    "cleanup plan row {} state key does not match current row",
                    snapshot.id
                );
            }
            _ => {}
        }
        if topic_group && snapshot.topic_key != current_snapshot.topic_key {
            bail!(
                "cleanup plan row {} topic key does not match current row",
                snapshot.id
            );
        }
    }
    Ok(())
}

fn insert_cleanup_operation_log(
    conn: &Connection,
    plan: &MemoryCleanupPlan,
    group: &MemoryCleanupGroup,
) -> Result<i64> {
    let current = snapshot_for(&group.row_snapshots, group.current_id)?;
    let mut operation_plan = MemoryOperationPlan::new(
        MemoryLifecycleOp::Update,
        group.state_key.clone(),
        group.reason.clone(),
    )
    .with_target_memory_id(Some(group.current_id))
    .with_superseded_ids(group.stale_ids.clone());
    operation_plan.planner_version = CLEANUP_PLANNER_VERSION;
    let input = MemoryOperationInput {
        source: "memory_cleanup".to_string(),
        actor: "memory_cleanup".to_string(),
        source_project: plan.project.clone(),
        owner_scope: group
            .owner_scope
            .clone()
            .unwrap_or_else(|| "repo".to_string()),
        owner_key: group
            .owner_key
            .clone()
            .unwrap_or_else(|| plan.project.clone()),
        memory_type: group.memory_type.clone(),
        topic_key: current.topic_key.clone(),
        state_key: group.state_key.clone(),
        source_candidate_id: None,
        confidence: Some(group.confidence),
    };
    insert_operation_log(conn, &input, &operation_plan, Some(group.current_id))
}

fn load_row_snapshots(conn: &Connection, ids: &[i64]) -> Result<Vec<MemoryCleanupRowSnapshot>> {
    ids.iter()
        .copied()
        .map(|id| {
            load_row_snapshot(conn, id)?
                .ok_or_else(|| anyhow!("cleanup plan target memory:{id} not found"))
        })
        .collect()
}

fn load_row_snapshot(conn: &Connection, id: i64) -> Result<Option<MemoryCleanupRowSnapshot>> {
    conn.query_row(
        "SELECT m.id, m.status, m.content, m.updated_at_epoch, m.owner_scope,
                m.owner_key, m.memory_type, m.topic_key, m.state_key_id, sk.state_key,
                sk.current_memory_id, m.project, m.scope, m.source_project, m.target_project
         FROM memories m
         LEFT JOIN memory_state_keys sk ON sk.id = m.state_key_id
         WHERE m.id = ?1",
        params![id],
        |row| {
            let content: String = row.get(2)?;
            Ok(MemoryCleanupRowSnapshot {
                id: row.get(0)?,
                status: row.get(1)?,
                content_sha256: content_sha256(&content),
                updated_at_epoch: row.get(3)?,
                owner_scope: row.get(4)?,
                owner_key: row.get(5)?,
                memory_type: row.get(6)?,
                topic_key: row.get(7)?,
                state_key_id: row.get(8)?,
                state_key: row.get(9)?,
                current_memory_id: row.get(10)?,
                project: row.get(11)?,
                scope: row.get(12)?,
                source_project: row.get(13)?,
                target_project: row.get(14)?,
            })
        },
    )
    .optional()
    .with_context(|| format!("load cleanup plan row snapshot for memory:{id}"))
}

impl MemoryCleanupRowSnapshot {
    fn owner_namespace(&self, project: &str) -> (String, String) {
        match (self.owner_scope.as_deref(), self.owner_key.as_deref()) {
            (Some(scope), Some(key)) => (scope.to_string(), key.to_string()),
            _ if self.project == project
                && self.scope.as_deref().unwrap_or("project") != "global" =>
            {
                ("legacy_repo".to_string(), project.to_string())
            }
            _ => ("legacy_other".to_string(), self.project.clone()),
        }
    }

    fn belongs_to_project(&self, project: &str) -> bool {
        self.source_project.as_deref() == Some(project)
            || self.target_project.as_deref() == Some(project)
            || (self.owner_scope.as_deref() == Some("repo")
                && self.owner_key.as_deref() == Some(project))
            || (self.owner_scope.is_none()
                && self.project == project
                && self.scope.as_deref().unwrap_or("project") != "global")
    }
}

fn snapshot_for(
    snapshots: &[MemoryCleanupRowSnapshot],
    id: i64,
) -> Result<&MemoryCleanupRowSnapshot> {
    snapshots
        .iter()
        .find(|snapshot| snapshot.id == id)
        .ok_or_else(|| anyhow!("cleanup plan missing snapshot for memory:{id}"))
}

fn content_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}
