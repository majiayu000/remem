use std::collections::HashSet;

use anyhow::{bail, Result};
use rusqlite::Connection;

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
