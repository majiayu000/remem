use std::collections::BTreeSet;

use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::{identity::ensure_workstream_alias, types::WorkStreamMergeResult};

pub fn merge_workstreams_manual(
    conn: &Connection,
    project: &str,
    canonical_id: i64,
    duplicate_ids: &[i64],
) -> Result<WorkStreamMergeResult> {
    if duplicate_ids.is_empty() {
        bail!("workstreams merge requires at least one duplicate id");
    }

    let mut unique_ids = BTreeSet::new();
    for duplicate_id in duplicate_ids {
        if *duplicate_id == canonical_id {
            bail!("workstream {canonical_id} cannot be merged into itself");
        }
        if !unique_ids.insert(*duplicate_id) {
            bail!("duplicate workstream id {duplicate_id} was provided more than once");
        }
    }

    let tx = conn.unchecked_transaction()?;
    let now = chrono::Utc::now().timestamp();
    let canonical = load_visible_merge_row(&tx, project, canonical_id)?.ok_or_else(|| {
        anyhow::anyhow!("No workstream found for id {canonical_id} in project {project}")
    })?;
    if canonical.merged_into_workstream_id.is_some() {
        bail!("canonical workstream {canonical_id} is already merged");
    }
    ensure_workstream_alias(
        &tx,
        canonical_id,
        &canonical.title,
        "manual_merge_canonical",
        None,
        None,
        now,
    )?;

    let mut moved_session_links = 0usize;
    let mut copied_aliases = 0usize;
    let mut copied_alias_sources = 0usize;

    for duplicate_id in duplicate_ids {
        let duplicate = load_visible_merge_row(&tx, project, *duplicate_id)?.ok_or_else(|| {
            anyhow::anyhow!("No workstream found for id {duplicate_id} in project {project}")
        })?;
        if duplicate.project != canonical.project {
            bail!(
                "workstream {duplicate_id} belongs to project {}, not {}",
                duplicate.project,
                canonical.project
            );
        }
        if duplicate.merged_into_workstream_id.is_some() {
            bail!("duplicate workstream {duplicate_id} is already merged");
        }

        let alias_rows = load_alias_rows(&tx, *duplicate_id)?;
        if alias_rows.is_empty() {
            ensure_workstream_alias(
                &tx,
                canonical_id,
                &duplicate.title,
                "manual_merge",
                None,
                Some(*duplicate_id),
                now,
            )?;
            copied_aliases += 1;
        }
        for alias in alias_rows {
            tx.execute(
                "INSERT INTO workstream_aliases
                 (workstream_id, title, normalized_title, first_seen_epoch, last_seen_epoch)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(workstream_id, normalized_title) DO UPDATE SET
                    first_seen_epoch = MIN(workstream_aliases.first_seen_epoch, excluded.first_seen_epoch),
                    last_seen_epoch = MAX(workstream_aliases.last_seen_epoch, excluded.last_seen_epoch)",
                params![
                    canonical_id,
                    alias.title,
                    alias.normalized_title,
                    alias.first_seen_epoch,
                    alias.last_seen_epoch,
                ],
            )?;
            copied_aliases += 1;

            let canonical_alias_id: i64 = tx.query_row(
                "SELECT id FROM workstream_aliases
                 WHERE workstream_id = ?1 AND normalized_title = ?2",
                params![canonical_id, alias.normalized_title],
                |row| row.get(0),
            )?;
            tx.execute(
                "INSERT INTO workstream_alias_sources
                 (alias_id, source, memory_session_id, source_workstream_id, observed_title,
                  first_seen_epoch, last_seen_epoch)
                 VALUES (?1, 'manual_merge', NULL, ?2, ?3, ?4, ?5)",
                params![
                    canonical_alias_id,
                    duplicate_id,
                    alias.title,
                    alias.first_seen_epoch,
                    alias.last_seen_epoch,
                ],
            )?;
            copied_alias_sources += 1;
            copied_alias_sources += tx.execute(
                "INSERT INTO workstream_alias_sources
                 (alias_id, source, memory_session_id, source_workstream_id, observed_title,
                  first_seen_epoch, last_seen_epoch)
                 SELECT ?1,
                        was.source,
                        was.memory_session_id,
                        COALESCE(was.source_workstream_id, ?3),
                        was.observed_title,
                        was.first_seen_epoch,
                        was.last_seen_epoch
                   FROM workstream_alias_sources was
                   JOIN workstream_aliases wa ON wa.id = was.alias_id
                  WHERE wa.workstream_id = ?3
                    AND wa.normalized_title = ?2",
                params![canonical_alias_id, alias.normalized_title, duplicate_id],
            )?;
        }

        moved_session_links += tx.execute(
            "INSERT OR IGNORE INTO workstream_sessions
             (workstream_id, memory_session_id, linked_at_epoch)
             SELECT ?1, memory_session_id, linked_at_epoch
               FROM workstream_sessions
              WHERE workstream_id = ?2",
            params![canonical_id, duplicate_id],
        )?;
        tx.execute(
            "DELETE FROM workstream_sessions WHERE workstream_id = ?1",
            params![duplicate_id],
        )?;
        tx.execute(
            "UPDATE workstreams
                SET merged_into_workstream_id = ?1,
                    updated_at_epoch = ?2
              WHERE id = ?3",
            params![canonical_id, now, duplicate_id],
        )?;
    }

    tx.execute(
        "UPDATE workstreams SET updated_at_epoch = ?1 WHERE id = ?2",
        params![now, canonical_id],
    )?;
    tx.commit()?;

    Ok(WorkStreamMergeResult {
        canonical_id,
        merged_ids: duplicate_ids.to_vec(),
        moved_session_links,
        copied_aliases,
        copied_alias_sources,
    })
}

#[derive(Debug)]
struct MergeRow {
    project: String,
    title: String,
    merged_into_workstream_id: Option<i64>,
}

fn load_visible_merge_row(
    conn: &Connection,
    project: &str,
    workstream_id: i64,
) -> Result<Option<MergeRow>> {
    conn.query_row(
        "SELECT project, title, merged_into_workstream_id
           FROM workstreams
          WHERE id = ?1
            AND ((owner_scope = 'repo' AND owner_key = ?2)
                 OR (owner_scope = 'repo' AND target_project = ?2)
                 OR (owner_scope = 'workstream' AND target_project = ?2)
                 OR (owner_scope IS NULL AND project = ?2))",
        params![workstream_id, project],
        |row| {
            Ok(MergeRow {
                project: row.get(0)?,
                title: row.get(1)?,
                merged_into_workstream_id: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

#[derive(Debug)]
struct AliasRow {
    title: String,
    normalized_title: String,
    first_seen_epoch: i64,
    last_seen_epoch: i64,
}

fn load_alias_rows(conn: &Connection, workstream_id: i64) -> Result<Vec<AliasRow>> {
    let mut stmt = conn.prepare(
        "SELECT title, normalized_title, first_seen_epoch, last_seen_epoch
           FROM workstream_aliases
          WHERE workstream_id = ?1
          ORDER BY first_seen_epoch ASC, id ASC",
    )?;
    let rows = stmt.query_map(params![workstream_id], |row| {
        Ok(AliasRow {
            title: row.get(0)?,
            normalized_title: row.get(1)?,
            first_seen_epoch: row.get(2)?,
            last_seen_epoch: row.get(3)?,
        })
    })?;
    crate::db::query::collect_rows(rows)
}
