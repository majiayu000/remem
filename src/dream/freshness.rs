use std::collections::HashSet;

use anyhow::{bail, Result};
use rusqlite::{params, Connection};

use super::candidates::Cluster;

pub(super) fn validate_cluster_snapshot(
    conn: &Connection,
    project: &str,
    cluster: &Cluster,
) -> Result<()> {
    if cluster.members.is_empty() {
        bail!("dream_cluster_snapshot_empty");
    }

    let current_filter =
        crate::memory::memory_current_filter_sql("m.status", "m.expires_at_epoch", false);
    let state_filter = crate::memory::memory_state_key_current_filter_sql("m");
    let policy_filter = crate::memory::suppression::memory_policy_filter_sql("m");
    let snapshot_query = format!(
        "SELECT EXISTS(
             SELECT 1
             FROM memories m
             WHERE m.id = ?1
               AND m.project = ?2
               AND {current_filter}
               AND {state_filter}
               AND {policy_filter}
               AND m.branch IS NULL
               AND m.version = ?3
               AND m.updated_at_epoch = ?4
               AND m.topic_key IS ?5
               AND m.title = ?6
               AND m.content = ?7
               AND m.memory_type = ?8
               AND COALESCE(
                    m.owner_scope,
                    CASE WHEN COALESCE(m.scope, 'project') = 'global' THEN 'user' ELSE 'repo' END
               ) = 'repo'
               AND COALESCE(
                    m.owner_key,
                    CASE WHEN COALESCE(m.scope, 'project') = 'global' THEN 'user:default' ELSE m.project END
               ) = ?2
         )"
    );
    let mut member_ids = HashSet::with_capacity(cluster.members.len());
    for member in &cluster.members {
        if !member_ids.insert(member.id) {
            bail!(
                "dream_cluster_snapshot_duplicate_member member_id={}",
                member.id
            );
        }

        let matches_snapshot: bool = conn.query_row(
            &snapshot_query,
            params![
                member.id,
                project,
                member.version,
                member.updated_at_epoch,
                member.topic_key.as_deref(),
                member.title.as_str(),
                member.content.as_str(),
                member.memory_type.as_str(),
            ],
            |row| row.get(0),
        )?;
        if !matches_snapshot {
            bail!("dream_cluster_snapshot_stale member_id={}", member.id);
        }
    }
    Ok(())
}
