use std::collections::HashSet;

use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub(super) fn superseded_topic_target(
    conn: &Connection,
    superseded_ids: &[i64],
    memory_type: &str,
    topic_key: &str,
) -> Result<Option<i64>> {
    let superseded_json = serde_json::to_string(superseded_ids)?;
    conn.query_row(
        "SELECT id FROM memories
         WHERE id IN (SELECT value FROM json_each(?1))
           AND memory_type = ?2 AND topic_key = ?3
           AND branch IS NULL AND COALESCE(scope, 'project') = 'project'
           AND COALESCE(owner_scope,
               CASE WHEN COALESCE(scope, 'project') = 'global'
                    THEN 'user' ELSE 'repo' END) = 'repo'
         ORDER BY CASE status WHEN 'active' THEN 0 ELSE 1 END,
                  updated_at_epoch DESC,
                  id DESC
         LIMIT 1",
        params![superseded_json, memory_type, topic_key],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn validate_cluster_superseded_ids(
    cluster: &super::super::Cluster,
    superseded_ids: &[i64],
) -> Result<()> {
    let member_ids = cluster
        .members
        .iter()
        .map(|member| member.id)
        .collect::<HashSet<_>>();
    if superseded_ids.is_empty() {
        bail!("dream merge requires at least one superseded cluster member");
    }
    if let Some(id) = superseded_ids.iter().find(|id| !member_ids.contains(id)) {
        bail!("dream superseded memory id={id} is outside cluster snapshot");
    }
    Ok(())
}

pub(super) struct TargetResolutionGuard {
    preexisting_ids: HashSet<i64>,
    allowed_ids: HashSet<i64>,
}

impl TargetResolutionGuard {
    pub(super) fn capture(conn: &Connection, allowed_ids: &[i64]) -> Result<Self> {
        let mut stmt = conn.prepare("SELECT id FROM memories")?;
        let preexisting_ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<HashSet<_>>>()?;
        Ok(Self {
            preexisting_ids,
            allowed_ids: allowed_ids.iter().copied().collect(),
        })
    }

    pub(super) fn validate_resolution(&self, memory_id: i64) -> Result<()> {
        if self.preexisting_ids.contains(&memory_id) && !self.allowed_ids.contains(&memory_id) {
            bail!("dream_target_resolution_outside_cluster memory_id={memory_id}");
        }
        Ok(())
    }
}
