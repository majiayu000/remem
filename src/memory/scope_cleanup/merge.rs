use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};
use serde::Serialize;

use super::audit::{load_memory_audit_rows, preference_clusters, DuplicateCluster};
use super::mutate::{insert_scope_cleanup_event, load_target, ObjectMutation, OwnerSnapshot};
use super::ObjectRef;

#[derive(Debug, Clone)]
pub struct MergePreferencesRequest<'a> {
    pub project: &'a str,
    pub dry_run: bool,
    pub confirm: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MergePreferencesResult {
    pub dry_run: bool,
    pub project: String,
    pub clusters: Vec<DuplicateCluster>,
    pub affected: Vec<ObjectMutation>,
}

pub fn merge_preferences(
    conn: &Connection,
    req: &MergePreferencesRequest<'_>,
) -> Result<MergePreferencesResult> {
    let dry_run = req.dry_run || !req.confirm;
    let memories = load_memory_audit_rows(conn, req.project, 500)?;
    let clusters = preference_clusters(&memories);
    let mut affected = Vec::new();
    if dry_run || clusters.is_empty() {
        return Ok(MergePreferencesResult {
            dry_run,
            project: req.project.to_string(),
            clusters,
            affected,
        });
    }

    let tx = conn.unchecked_transaction()?;
    let now = chrono::Utc::now().timestamp();
    for cluster in &clusters {
        let canonical_ref = ObjectRef::parse(&cluster.canonical_ref)?;
        let canonical = load_target(&tx, canonical_ref)?;
        let merged = cluster
            .merged_content
            .as_deref()
            .ok_or_else(|| anyhow!("duplicate preference cluster missing merged content"))?;
        tx.execute(
            "UPDATE memories
             SET content = ?1,
                 status = 'active',
                 source_project = COALESCE(source_project, project),
                 target_project = ?2,
                 owner_scope = 'repo',
                 owner_key = ?2,
                 updated_at_epoch = ?3
             WHERE id = ?4",
            params![merged, req.project, now, canonical_ref.id],
        )?;
        insert_scope_cleanup_event(
            &tx,
            "merge-preferences",
            &canonical,
            "active",
            &canonical.owner,
            Some("canonical preference updated with merged duplicate content"),
            now,
        )?;

        for object_ref in &cluster.refs {
            if object_ref == &cluster.canonical_ref {
                continue;
            }
            let object_ref = ObjectRef::parse(object_ref)?;
            let target = load_target(&tx, object_ref)?;
            tx.execute(
                "UPDATE memories SET status = 'stale', updated_at_epoch = ?1 WHERE id = ?2",
                params![now, object_ref.id],
            )?;
            let new_owner = OwnerSnapshot {
                source_project: target
                    .owner
                    .source_project
                    .clone()
                    .or_else(|| target.project.clone()),
                target_project: Some(req.project.to_string()),
                owner_scope: Some("repo".to_string()),
                owner_key: Some(req.project.to_string()),
                topic_domain: target.owner.topic_domain.clone(),
                routing_confidence: target.owner.routing_confidence,
                routing_reason: Some("superseded by merged preference".to_string()),
                context_class: target.owner.context_class.clone(),
            };
            affected.push(ObjectMutation {
                object_ref: object_ref.to_string(),
                title: target.title.clone(),
                previous_status: target.status.clone(),
                new_status: "stale".to_string(),
                previous_owner: target.owner.clone(),
                new_owner: new_owner.clone(),
            });
            insert_scope_cleanup_event(
                &tx,
                "merge-preferences",
                &target,
                "stale",
                &new_owner,
                Some("duplicate preference superseded by canonical row"),
                now,
            )?;
        }
    }
    tx.commit()?;
    Ok(MergePreferencesResult {
        dry_run,
        project: req.project.to_string(),
        clusters,
        affected,
    })
}
