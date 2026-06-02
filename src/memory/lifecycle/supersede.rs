use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};

pub fn soft_supersede(
    conn: &Connection,
    project: &str,
    memory_ids: &[i64],
    replacement_id: Option<i64>,
) -> Result<usize> {
    let mut seen = std::collections::HashSet::with_capacity(memory_ids.len());
    let targets = memory_ids
        .iter()
        .copied()
        .filter(|id| Some(*id) != replacement_id && seen.insert(*id))
        .collect::<Vec<_>>();
    for id in &targets {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM memories WHERE id = ?1 AND project = ?2)",
            params![id, project],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(anyhow!(
                "failed to mark superseded memory stale: id={} project={}",
                id,
                project
            ));
        }
    }

    let mut changed = 0usize;
    let now = chrono::Utc::now().timestamp();
    for id in targets {
        let updated = conn.execute(
            "UPDATE memories
             SET status = 'stale',
                 valid_to_epoch = COALESCE(valid_to_epoch, ?3)
             WHERE id = ?1 AND project = ?2",
            params![id, project, now],
        )?;
        if updated != 1 {
            return Err(anyhow!(
                "failed to mark superseded memory stale: id={} project={}",
                id,
                project
            ));
        }
        changed += updated;
    }
    Ok(changed)
}

pub(super) fn soft_supersede_owned(
    conn: &Connection,
    ownership: &super::LifecycleOwnership<'_>,
    memory_ids: &[i64],
    replacement_id: Option<i64>,
) -> Result<usize> {
    let mut seen = std::collections::HashSet::with_capacity(memory_ids.len());
    let targets = memory_ids
        .iter()
        .copied()
        .filter(|id| Some(*id) != replacement_id && seen.insert(*id))
        .collect::<Vec<_>>();
    for id in &targets {
        if !exists_owned(conn, *id, ownership)? {
            return Err(owner_error(*id, ownership));
        }
    }

    let mut changed = 0usize;
    let now = chrono::Utc::now().timestamp();
    for id in targets {
        let updated = conn.execute(
            "UPDATE memories
             SET status = 'stale',
                 valid_to_epoch = COALESCE(valid_to_epoch, ?4)
             WHERE id = ?1
               AND COALESCE(
                    owner_scope,
                    CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user' ELSE 'repo' END
               ) = ?2
               AND COALESCE(
                    owner_key,
                    CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user:default' ELSE project END
               ) = ?3",
            params![id, ownership.owner_scope, ownership.owner_key, now],
        )?;
        if updated != 1 {
            return Err(owner_error(id, ownership));
        }
        changed += updated;
    }
    Ok(changed)
}

fn exists_owned(
    conn: &Connection,
    id: i64,
    ownership: &super::LifecycleOwnership<'_>,
) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM memories
             WHERE id = ?1
               AND COALESCE(
                    owner_scope,
                    CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user' ELSE 'repo' END
               ) = ?2
               AND COALESCE(
                    owner_key,
                    CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user:default' ELSE project END
               ) = ?3
         )",
        params![id, ownership.owner_scope, ownership.owner_key],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn owner_error(id: i64, ownership: &super::LifecycleOwnership<'_>) -> anyhow::Error {
    anyhow!(
        "failed to mark superseded memory stale: id={} owner_scope={} owner_key={}",
        id,
        ownership.owner_scope,
        ownership.owner_key
    )
}
