use std::collections::BTreeSet;

use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::{ActiveTopicMemory, CandidateRoute, ParsedMemoryCandidate};

pub(super) fn load_required_memories(
    conn: &Connection,
    candidate: &ParsedMemoryCandidate,
    route: &CandidateRoute,
    memory_project: &str,
    required_ids: &BTreeSet<i64>,
    now_epoch: i64,
    candidate_has_ttl: bool,
) -> Result<Vec<ActiveTopicMemory>> {
    if required_ids.is_empty() {
        bail!("Dream promotion requires at least one reviewed supersede target");
    }

    let mut memories = Vec::with_capacity(required_ids.len());
    for id in required_ids {
        let row = conn
            .query_row(
                "SELECT project, memory_type, content, status,
                        COALESCE(
                            owner_scope,
                            CASE WHEN COALESCE(scope, 'project') = 'global'
                                 THEN 'user' ELSE 'repo' END
                        ),
                        COALESCE(
                            owner_key,
                            CASE WHEN COALESCE(scope, 'project') = 'global'
                                 THEN 'user:default' ELSE project END
                        ),
                        expires_at_epoch
                 FROM memories WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((project, memory_type, content, status, owner_scope, owner_key, expires_at)) = row
        else {
            bail!("reviewed Dream supersede target is missing: id={id}");
        };
        if project != memory_project
            || memory_type != candidate.memory_type
            || owner_scope != route.owner_scope
            || owner_key != route.owner_key
        {
            bail!("reviewed Dream supersede target left its project/type/owner scope: id={id}");
        }
        if status != "active" {
            bail!("reviewed Dream supersede target is no longer active: id={id}");
        }
        if !memory_is_canonically_current(conn, *id)? {
            bail!("reviewed Dream supersede target is no longer current: id={id}");
        }
        let is_current = if candidate_has_ttl {
            expires_at.is_some_and(|epoch| epoch > now_epoch)
        } else {
            expires_at.is_none_or(|epoch| epoch > now_epoch)
        };
        if !is_current {
            bail!("reviewed Dream supersede target is no longer current: id={id}");
        }
        memories.push(ActiveTopicMemory {
            id: *id,
            content,
            is_current: true,
        });
    }
    Ok(memories)
}

fn memory_is_canonically_current(conn: &Connection, memory_id: i64) -> Result<bool> {
    let current_filter =
        crate::memory::memory_current_filter_sql("m.status", "m.expires_at_epoch", false);
    let state_filter = crate::memory::memory_state_key_current_filter_sql("m");
    let policy_filter = crate::memory::suppression::memory_policy_filter_sql("m");
    let sql = format!(
        "SELECT EXISTS(
             SELECT 1 FROM memories m
             WHERE m.id = ?1
               AND {current_filter}
               AND {state_filter}
               AND {policy_filter}
         )"
    );
    conn.query_row(&sql, params![memory_id], |row| row.get::<_, i64>(0))
        .map(|exists| exists != 0)
        .map_err(Into::into)
}
