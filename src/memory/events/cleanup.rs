use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::db::{self, CompressedObservationSource, Observation};

pub const OLD_EVENT_RETENTION_DAYS: i64 = 30;
pub const COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS: i64 = 90;
pub const STALE_MEMORY_ARCHIVE_DAYS: i64 = 180;

const SECONDS_PER_DAY: i64 = 86_400;

pub fn cleanup_old_events(conn: &Connection, days: i64) -> Result<usize> {
    cleanup_old_events_at(conn, chrono::Utc::now().timestamp(), days)
}

pub fn count_old_events(conn: &Connection, days: i64) -> Result<usize> {
    count_old_events_at(conn, chrono::Utc::now().timestamp(), days)
}

pub fn cleanup_old_events_at(conn: &Connection, now_epoch: i64, days: i64) -> Result<usize> {
    let cutoff = cutoff_epoch(now_epoch, days);
    Ok(conn.execute(
        "DELETE FROM events WHERE created_at_epoch < ?1",
        params![cutoff],
    )?)
}

pub fn count_old_events_at(conn: &Connection, now_epoch: i64, days: i64) -> Result<usize> {
    let cutoff = cutoff_epoch(now_epoch, days);
    count_rows(
        conn,
        "SELECT COUNT(*) FROM events WHERE created_at_epoch < ?1",
        &[&cutoff],
    )
}

pub fn archive_stale_memories(conn: &Connection, days: i64) -> Result<usize> {
    archive_stale_memories_at(conn, chrono::Utc::now().timestamp(), days)
}

pub fn count_stale_memories_to_archive(conn: &Connection, days: i64) -> Result<usize> {
    count_stale_memories_to_archive_at(conn, chrono::Utc::now().timestamp(), days)
}

pub fn archive_stale_memories_at(conn: &Connection, now_epoch: i64, days: i64) -> Result<usize> {
    let cutoff = cutoff_epoch(now_epoch, days);
    Ok(conn.execute(
        "UPDATE memories SET status = 'archived' \
         WHERE status = 'stale' AND updated_at_epoch < ?1",
        params![cutoff],
    )?)
}

pub fn count_stale_memories_to_archive_at(
    conn: &Connection,
    now_epoch: i64,
    days: i64,
) -> Result<usize> {
    let cutoff = cutoff_epoch(now_epoch, days);
    count_rows(
        conn,
        "SELECT COUNT(*) FROM memories WHERE status = 'stale' AND updated_at_epoch < ?1",
        &[&cutoff],
    )
}

pub fn count_compressed_source_observations_to_delete(
    conn: &Connection,
    days: i64,
) -> Result<usize> {
    count_compressed_source_observations_to_delete_at(conn, chrono::Utc::now().timestamp(), days)
}

pub fn count_compressed_source_observations_to_delete_at(
    conn: &Connection,
    now_epoch: i64,
    days: i64,
) -> Result<usize> {
    Ok(compressed_source_observation_ids_to_delete_at(conn, now_epoch, days)?.len())
}

pub fn cleanup_compressed_source_observations(conn: &Connection, days: i64) -> Result<usize> {
    cleanup_compressed_source_observations_at(conn, chrono::Utc::now().timestamp(), days)
}

pub fn cleanup_compressed_source_observations_at(
    conn: &Connection,
    now_epoch: i64,
    days: i64,
) -> Result<usize> {
    let ids = compressed_source_observation_ids_to_delete_at(conn, now_epoch, days)?;
    delete_observations_by_ids(conn, &ids)
}

pub fn compressed_source_observation_ids_to_delete_at(
    conn: &Connection,
    now_epoch: i64,
    days: i64,
) -> Result<Vec<i64>> {
    let cutoff = cutoff_epoch(now_epoch, days);
    let mut stmt = conn.prepare(
        "SELECT id, memory_session_id, type, title, subtitle, narrative,
                facts, concepts, files_read, files_modified, discovery_tokens,
                created_at, created_at_epoch, project, status, last_accessed_epoch,
                (SELECT s.content_session_id FROM sdk_sessions s
                 WHERE s.memory_session_id = o.memory_session_id LIMIT 1)
                 AS content_session_id,
                branch, commit_sha
         FROM observations o
         WHERE o.status = 'compressed'
           AND o.created_at_epoch < 10000000000
           AND EXISTS (
               SELECT 1 FROM compressed_observation_sources source_link
               WHERE source_link.source_observation_id = o.id
                 AND source_link.created_at_epoch < ?1
           )
           AND NOT EXISTS (
               SELECT 1 FROM compressed_observation_sources owned
               WHERE owned.compressed_observation_id = o.id
           )
         ORDER BY o.created_at_epoch ASC, o.id ASC",
    )?;
    let rows = stmt.query_map(params![cutoff], map_observation_row)?;

    let mut ids = Vec::new();
    for row in rows {
        let source = row?;
        if has_sufficient_compression_provenance(conn, &source, cutoff)? {
            ids.push(source.id);
        }
    }
    Ok(ids)
}

fn has_sufficient_compression_provenance(
    conn: &Connection,
    source: &Observation,
    cutoff_epoch: i64,
) -> Result<bool> {
    let links = load_links_for_source(conn, source.id)?;
    if links.is_empty() {
        return Ok(false);
    }

    let expected_hash = db::observation_source_hash(source);
    for link in links {
        if link.created_at_epoch >= cutoff_epoch {
            continue;
        }
        if link.source_hash != expected_hash {
            continue;
        }
        if link.source_created_at_epoch != source.created_at_epoch {
            continue;
        }
        if !snapshot_matches_source(&link.source_snapshot_json, source)? {
            continue;
        }
        if compressed_observation_exists(conn, link.compressed_observation_id, source.id)? {
            return Ok(true);
        }
    }

    Ok(false)
}

fn load_links_for_source(
    conn: &Connection,
    source_observation_id: i64,
) -> Result<Vec<CompressedObservationSource>> {
    let mut stmt = conn.prepare(
        "SELECT compressed_observation_id, source_observation_id, source_hash,
                source_snapshot_json, source_created_at_epoch, compression_session_id,
                created_at_epoch
         FROM compressed_observation_sources
         WHERE source_observation_id = ?1
         ORDER BY compressed_observation_id",
    )?;
    let rows = stmt.query_map(params![source_observation_id], |row| {
        Ok(CompressedObservationSource {
            compressed_observation_id: row.get(0)?,
            source_observation_id: row.get(1)?,
            source_hash: row.get(2)?,
            source_snapshot_json: row.get(3)?,
            source_created_at_epoch: row.get(4)?,
            compression_session_id: row.get(5)?,
            created_at_epoch: row.get(6)?,
        })
    })?;

    let mut links = Vec::new();
    for row in rows {
        links.push(row?);
    }
    Ok(links)
}

fn snapshot_matches_source(snapshot_json: &str, source: &Observation) -> Result<bool> {
    let snapshot: serde_json::Value =
        serde_json::from_str(snapshot_json).context("invalid compressed source snapshot JSON")?;
    Ok(snapshot
        .get("hash_version")
        .and_then(|value| value.as_str())
        == Some("observation-v1")
        && snapshot.get("id").and_then(|value| value.as_i64()) == Some(source.id)
        && snapshot
            .get("created_at_epoch")
            .and_then(|value| value.as_i64())
            == Some(source.created_at_epoch))
}

fn compressed_observation_exists(
    conn: &Connection,
    compressed_observation_id: i64,
    source_observation_id: i64,
) -> Result<bool> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM observations
             WHERE id = ?1 AND id != ?2
         )",
        params![compressed_observation_id, source_observation_id],
        |row| row.get(0),
    )?;
    Ok(exists)
}

fn delete_observations_by_ids(conn: &Connection, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }

    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "DELETE FROM observations WHERE id IN ({})",
        placeholders.join(", ")
    );
    let params: Vec<Box<dyn rusqlite::types::ToSql>> = ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    let refs = crate::db::to_sql_refs(&params);
    Ok(conn.execute(&sql, refs.as_slice())?)
}

fn count_rows(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::types::ToSql],
) -> Result<usize> {
    let count: i64 = conn.query_row(sql, params, |row| row.get(0))?;
    Ok(count as usize)
}

fn cutoff_epoch(now_epoch: i64, days: i64) -> i64 {
    now_epoch.saturating_sub(days.saturating_mul(SECONDS_PER_DAY))
}

fn map_observation_row(row: &rusqlite::Row) -> rusqlite::Result<Observation> {
    Ok(Observation {
        id: row.get(0)?,
        memory_session_id: row.get(1)?,
        r#type: row.get(2)?,
        title: row.get(3)?,
        subtitle: row.get(4)?,
        narrative: row.get(5)?,
        facts: row.get(6)?,
        concepts: row.get(7)?,
        files_read: row.get(8)?,
        files_modified: row.get(9)?,
        discovery_tokens: row.get(10)?,
        created_at: row.get(11)?,
        created_at_epoch: row.get(12)?,
        project: row.get(13)?,
        status: row
            .get::<_, Option<String>>(14)?
            .unwrap_or_else(|| "active".to_string()),
        last_accessed_epoch: row.get(15)?,
        content_session_id: row.get(16)?,
        branch: row.get(17)?,
        commit_sha: row.get(18)?,
    })
}
