use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::db::{self, CompressedObservationSource, Observation};

pub const OLD_EVENT_RETENTION_DAYS: i64 = 30;
pub const COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS: i64 = 90;
pub const STALE_MEMORY_ARCHIVE_DAYS: i64 = 180;

const SECONDS_PER_DAY: i64 = 86_400;
const COMPRESSED_SOURCE_SCAN_BATCH_SIZE: i64 = 500;

pub fn cleanup_old_events(conn: &Connection, days: i64) -> Result<usize> {
    cleanup_old_events_at(conn, chrono::Utc::now().timestamp(), days)
}

pub fn count_old_events(conn: &Connection, days: i64) -> Result<usize> {
    count_old_events_at(conn, chrono::Utc::now().timestamp(), days)
}

pub fn cleanup_old_events_at(conn: &Connection, now_epoch: i64, days: i64) -> Result<usize> {
    let Some(has_audit_references) = event_retention_schema(conn)? else {
        return Ok(0);
    };
    let cutoff = cutoff_epoch(now_epoch, days);
    let sql = if has_audit_references {
        "DELETE FROM events
         WHERE retention_class = 'ephemeral'
           AND created_at_epoch < ?1
           AND NOT EXISTS (
             SELECT 1 FROM api_mutation_requests request
             WHERE request.audit_id = events.id
           )"
    } else {
        "DELETE FROM events
         WHERE retention_class = 'ephemeral'
           AND created_at_epoch < ?1"
    };
    Ok(conn.execute(sql, params![cutoff])?)
}

pub fn count_old_events_at(conn: &Connection, now_epoch: i64, days: i64) -> Result<usize> {
    let Some(has_audit_references) = event_retention_schema(conn)? else {
        return Ok(0);
    };
    let cutoff = cutoff_epoch(now_epoch, days);
    let sql = if has_audit_references {
        "SELECT COUNT(*) FROM events
         WHERE retention_class = 'ephemeral'
           AND created_at_epoch < ?1
           AND NOT EXISTS (
             SELECT 1 FROM api_mutation_requests request
             WHERE request.audit_id = events.id
           )"
    } else {
        "SELECT COUNT(*) FROM events
         WHERE retention_class = 'ephemeral'
           AND created_at_epoch < ?1"
    };
    count_rows(conn, sql, &[&cutoff])
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
    let mut count = 0;
    visit_compressed_source_observations_to_delete_at(conn, now_epoch, days, false, |_| {
        count += 1;
        Ok(())
    })?;
    Ok(count)
}

pub fn cleanup_compressed_source_observations(conn: &Connection, days: i64) -> Result<usize> {
    cleanup_compressed_source_observations_at(conn, chrono::Utc::now().timestamp(), days)
}

pub fn cleanup_compressed_source_observations_at(
    conn: &Connection,
    now_epoch: i64,
    days: i64,
) -> Result<usize> {
    if !conn.is_autocommit() {
        return cleanup_compressed_sources_in_transaction(conn, now_epoch, days);
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .context("begin compressed source cleanup transaction")?;
    let deleted = cleanup_compressed_sources_in_transaction(&tx, now_epoch, days)?;
    tx.commit()
        .context("commit compressed source cleanup transaction")?;
    Ok(deleted)
}

fn cleanup_compressed_sources_in_transaction(
    conn: &Connection,
    now_epoch: i64,
    days: i64,
) -> Result<usize> {
    let mut deleted = 0;
    visit_compressed_source_observations_to_delete_at(conn, now_epoch, days, true, |id| {
        deleted += conn.execute(
            "DELETE FROM observations
                 WHERE id = ?1 AND status = 'compressed'
                   AND NOT EXISTS (
                     SELECT 1 FROM compressed_observation_sources owned
                     WHERE owned.compressed_observation_id = observations.id
                 )",
            params![id],
        )?;
        Ok(())
    })?;
    Ok(deleted)
}

pub fn compressed_source_observation_ids_to_delete_at(
    conn: &Connection,
    now_epoch: i64,
    days: i64,
) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    visit_compressed_source_observations_to_delete_at(conn, now_epoch, days, false, |id| {
        ids.push(id);
        Ok(())
    })?;
    Ok(ids)
}

fn visit_compressed_source_observations_to_delete_at(
    conn: &Connection,
    now_epoch: i64,
    days: i64,
    upgrade_legacy_links: bool,
    mut visit: impl FnMut(i64) -> Result<()>,
) -> Result<()> {
    db::ensure_observation_retention_schema_supported(conn)?;
    let cutoff = cutoff_epoch(now_epoch, days);
    let mut after_created_at_epoch = i64::MIN;
    let mut after_id = i64::MIN;
    loop {
        let batch = {
            let mut stmt = conn.prepare(
                "SELECT o.id, o.created_at_epoch
                 FROM observations o
                 WHERE o.status = 'compressed'
                   AND o.created_at_epoch < 10000000000
                   AND (
                       o.created_at_epoch > ?2
                       OR (o.created_at_epoch = ?2 AND o.id > ?3)
                   )
                   AND EXISTS (
                       SELECT 1 FROM compressed_observation_sources source_link
                       WHERE source_link.source_observation_id = o.id
                         AND source_link.created_at_epoch < ?1
                   )
                   AND NOT EXISTS (
                       SELECT 1 FROM compressed_observation_sources owned
                       WHERE owned.compressed_observation_id = o.id
                   )
                 ORDER BY o.created_at_epoch ASC, o.id ASC
                 LIMIT ?4",
            )?;
            let rows = stmt.query_map(
                params![
                    cutoff,
                    after_created_at_epoch,
                    after_id,
                    COMPRESSED_SOURCE_SCAN_BATCH_SIZE
                ],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )?;
            crate::db::query::collect_rows(rows)?
        };
        let Some(&(last_id, last_created_at_epoch)) = batch.last() else {
            break;
        };
        after_created_at_epoch = last_created_at_epoch;
        after_id = last_id;
        for (id, _) in batch {
            if compressed_source_is_delete_eligible(conn, id, cutoff, upgrade_legacy_links)? {
                visit(id)?;
            }
        }
    }
    Ok(())
}

fn compressed_source_is_delete_eligible(
    conn: &Connection,
    source_id: i64,
    cutoff_epoch: i64,
    upgrade_legacy_links: bool,
) -> Result<bool> {
    let Some(source) = load_observation(conn, source_id)? else {
        return Ok(false);
    };
    if source.status != "compressed"
        || source.created_at_epoch >= 10_000_000_000
        || source_has_memory_fact_reference(conn, source_id)?
    {
        return Ok(false);
    }
    let owned: bool = conn.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM compressed_observation_sources
           WHERE compressed_observation_id = ?1
         )",
        params![source_id],
        |row| row.get(0),
    )?;
    Ok(!owned
        && has_sufficient_compression_provenance(
            conn,
            &source,
            cutoff_epoch,
            upgrade_legacy_links,
        )?)
}

fn load_observation(conn: &Connection, id: i64) -> Result<Option<Observation>> {
    conn.query_row(
        "SELECT id, memory_session_id, type, title, subtitle, narrative,
                facts, concepts, files_read, files_modified, discovery_tokens,
                created_at, created_at_epoch, project, status, last_accessed_epoch,
                (SELECT s.content_session_id FROM sdk_sessions s
                 WHERE s.memory_session_id = o.memory_session_id LIMIT 1),
                branch, commit_sha
         FROM observations o
         WHERE o.id = ?1",
        params![id],
        map_observation_row,
    )
    .optional()
    .context("load compressed source observation")
}

fn source_has_unhashed_provenance(conn: &Connection, source_id: i64) -> Result<bool> {
    let candidates = [
        ("prompt_number", "prompt_number IS NOT NULL"),
        ("last_accessed_epoch", "last_accessed_epoch IS NOT NULL"),
        ("host_id", "host_id IS NOT NULL"),
        ("project_id", "project_id IS NOT NULL"),
        ("session_row_id", "session_row_id IS NOT NULL"),
        (
            "observation_type",
            "NULLIF(TRIM(observation_type), '') IS NOT NULL",
        ),
        ("text", "NULLIF(TRIM(text), '') IS NOT NULL"),
        (
            "evidence_event_ids",
            "COALESCE(NULLIF(TRIM(evidence_event_ids), ''), '[]') NOT IN ('[]', 'null')",
        ),
        ("confidence", "confidence IS NOT NULL"),
        ("reference_time_epoch", "reference_time_epoch IS NOT NULL"),
    ];
    let mut predicates = Vec::new();
    for (column, predicate) in candidates {
        if table_column_exists(conn, "observations", column)? {
            predicates.push(predicate);
        }
    }
    if predicates.is_empty() {
        return Ok(false);
    }
    let sql = format!(
        "SELECT EXISTS(SELECT 1 FROM observations WHERE id = ?1 AND ({}))",
        predicates.join(" OR ")
    );
    Ok(conn.query_row(&sql, params![source_id], |row| row.get(0))?)
}

fn source_has_memory_fact_reference(conn: &Connection, source_id: i64) -> Result<bool> {
    if !table_exists(conn, "memory_facts")? {
        return Ok(false);
    }
    if !table_column_exists(conn, "memory_facts", "source_observation_id")? {
        return Ok(true);
    }
    Ok(conn.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM memory_facts WHERE source_observation_id = ?1
         )",
        params![source_id],
        |row| row.get(0),
    )?)
}

fn has_sufficient_compression_provenance(
    conn: &Connection,
    source: &Observation,
    cutoff_epoch: i64,
    upgrade_legacy_links: bool,
) -> Result<bool> {
    let links = load_links_for_source(conn, source.id)?;
    if links.is_empty() {
        return Ok(false);
    }

    let has_v1_link = links
        .iter()
        .any(|link| link.source_hash.starts_with("sha256:observation-v1:"));
    let has_v2_link = links
        .iter()
        .any(|link| link.source_hash.starts_with("sha256:observation-v2:"));
    let legacy_has_unhashed_provenance = if has_v1_link {
        source_has_unhashed_provenance(conn, source.id)?
    } else {
        false
    };
    let legacy_expected_hash = has_v1_link.then(|| db::observation_source_hash(source));
    let current_record = if has_v2_link || (has_v1_link && legacy_has_unhashed_provenance) {
        Some(db::observation_source_retention_record_on_supported_schema(
            conn, source,
        )?)
    } else {
        None
    };
    for link in links {
        if link.created_at_epoch >= cutoff_epoch {
            continue;
        }
        if link.source_created_at_epoch != source.created_at_epoch {
            continue;
        }
        let is_legacy_link = link.source_hash.starts_with("sha256:observation-v1:");
        let matches_supported_snapshot = if link.source_hash.starts_with("sha256:observation-v2:") {
            snapshot_json_is_valid(&link.source_snapshot_json, source.id, "v2")
                && current_record.as_ref().is_some_and(|record| {
                    link.source_hash == record.source_hash
                        && link.source_snapshot_json == record.source_snapshot_json
                })
        } else if is_legacy_link {
            legacy_expected_hash
                .as_ref()
                .is_some_and(|expected| link.source_hash == *expected)
                && snapshot_matches_source(&link.source_snapshot_json, source)?
        } else {
            false
        };
        if !matches_supported_snapshot {
            continue;
        }
        if compressed_observation_exists(conn, link.compressed_observation_id, source.id)? {
            if is_legacy_link && legacy_has_unhashed_provenance && upgrade_legacy_links {
                let record = current_record
                    .as_ref()
                    .context("build v2 record for legacy compressed source upgrade")?;
                upgrade_legacy_source_link(conn, &link, record)?;
            }
            return Ok(true);
        }
    }

    Ok(false)
}

fn upgrade_legacy_source_link(
    conn: &Connection,
    link: &CompressedObservationSource,
    record: &db::ObservationSourceRetentionRecord,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE compressed_observation_sources
         SET source_hash = ?1, source_snapshot_json = ?2
         WHERE compressed_observation_id = ?3
           AND source_observation_id = ?4
           AND source_hash = ?5
           AND source_snapshot_json = ?6",
        params![
            record.source_hash,
            record.source_snapshot_json,
            link.compressed_observation_id,
            link.source_observation_id,
            link.source_hash,
            link.source_snapshot_json
        ],
    )?;
    if changed != 1 {
        anyhow::bail!(
            "legacy compressed source provenance changed during upgrade: source_id={}",
            link.source_observation_id
        );
    }
    Ok(())
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
    if !snapshot_json_is_valid(snapshot_json, source.id, "v1") {
        return Ok(false);
    }
    let expected = db::observation_source_snapshot_json(source)
        .context("build expected compressed source snapshot")?;
    Ok(snapshot_json == expected)
}

fn snapshot_json_is_valid(snapshot_json: &str, source_id: i64, version: &str) -> bool {
    if serde_json::from_str::<serde_json::Value>(snapshot_json).is_ok() {
        return true;
    }
    crate::log::error(
        "cleanup",
        &format!(
            "ignoring invalid compressed source {version} snapshot for observation {source_id}"
        ),
    );
    false
}

fn compressed_observation_exists(
    conn: &Connection,
    compressed_observation_id: i64,
    source_observation_id: i64,
) -> Result<bool> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM observations
             WHERE id = ?1 AND id != ?2 AND status = 'active'
         )",
        params![compressed_observation_id, source_observation_id],
        |row| row.get(0),
    )?;
    Ok(exists)
}

fn count_rows(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::types::ToSql],
) -> Result<usize> {
    let count: i64 = conn.query_row(sql, params, |row| row.get(0))?;
    Ok(count as usize)
}

/// `None` means the retention discriminator is unavailable or inconsistent, so
/// cleanup must fail closed instead of falling back to age-only deletion.
fn event_retention_schema(conn: &Connection) -> Result<Option<bool>> {
    if !table_column_exists(conn, "events", "retention_class")? {
        return Ok(None);
    }
    if !table_exists(conn, "api_mutation_requests")? {
        return Ok(Some(false));
    }
    if !table_column_exists(conn, "api_mutation_requests", "audit_id")? {
        return Ok(None);
    }
    Ok(Some(true))
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn table_column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    if !table_exists(conn, table)? {
        return Ok(false);
    }
    let sql = format!(
        "SELECT 1 FROM pragma_table_info('{}') WHERE name = ?1",
        table
    );
    Ok(conn
        .query_row(&sql, params![column], |_| Ok(()))
        .optional()?
        .is_some())
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
