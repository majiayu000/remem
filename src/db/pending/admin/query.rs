use anyhow::{ensure, Result};
use rusqlite::{params, Connection};

use super::types::{
    AdminRequiredArchivedLegacyPendingRow, ArchivedTransientLegacyPendingStats, FailedPendingRow,
};

pub fn list_failed(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<FailedPendingRow>> {
    let limit = limit.max(1);
    let mut rows_out = Vec::new();

    if let Some(project) = project {
        let mut stmt = conn.prepare(
            "SELECT id, session_id, project, tool_name, attempt_count, updated_at_epoch, last_error
             FROM pending_observations
             WHERE status = 'failed' AND project = ?1
             ORDER BY updated_at_epoch DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![project, limit], FailedPendingRow::from_row)?;
        for row in rows {
            rows_out.push(row?);
        }
        return Ok(rows_out);
    }

    let mut stmt = conn.prepare(
        "SELECT id, session_id, project, tool_name, attempt_count, updated_at_epoch, last_error
         FROM pending_observations
         WHERE status = 'failed'
         ORDER BY updated_at_epoch DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], FailedPendingRow::from_row)?;
    for row in rows {
        rows_out.push(row?);
    }
    Ok(rows_out)
}

pub(crate) fn list_admin_required_archived_legacy_pending(
    conn: &Connection,
    limit: i64,
) -> Result<Vec<AdminRequiredArchivedLegacyPendingRow>> {
    let limit = limit.max(1);
    let mut stmt = conn.prepare(
        "SELECT id, host, failure_class, archived_at_epoch
         FROM pending_observations
         WHERE status = 'failed'
           AND archived_at_epoch IS NOT NULL
           AND NOT (
               host IN (?1, ?2)
               AND COALESCE(failure_class, 'transient') = 'transient'
           )
         ORDER BY archived_at_epoch ASC, id ASC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        params![
            crate::runtime_config::CLAUDE_HOST,
            crate::runtime_config::CODEX_HOST,
            limit
        ],
        AdminRequiredArchivedLegacyPendingRow::from_row,
    )?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub(crate) fn query_archived_transient_legacy_pending(
    conn: &Connection,
) -> Result<ArchivedTransientLegacyPendingStats> {
    let now = chrono::Utc::now().timestamp();
    let (due, deferred, earliest_deferred_retry_epoch): (i64, i64, Option<i64>) = conn.query_row(
        "SELECT
             COALESCE(SUM(CASE
                 WHEN next_retry_epoch IS NULL OR next_retry_epoch <= ?3 THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN next_retry_epoch > ?3 THEN 1 ELSE 0 END), 0),
             MIN(CASE WHEN next_retry_epoch > ?3 THEN next_retry_epoch END)
         FROM pending_observations
         WHERE host IN (?1, ?2)
           AND status = 'failed'
           AND COALESCE(failure_class, 'transient') = 'transient'
           AND archived_at_epoch IS NOT NULL",
        params![
            crate::runtime_config::CLAUDE_HOST,
            crate::runtime_config::CODEX_HOST,
            now,
        ],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    ensure!(
        deferred == 0 || earliest_deferred_retry_epoch.is_some(),
        "deferred archived transient pending observations have no next retry epoch"
    );
    Ok(ArchivedTransientLegacyPendingStats {
        due: due.max(0) as usize,
        deferred: deferred.max(0) as usize,
        earliest_deferred_retry_epoch,
    })
}

pub fn count_failed_retry_candidates(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<usize> {
    let limit = limit.max(1);
    let count: i64 = if let Some(project) = project {
        conn.query_row(
            "SELECT COUNT(*) FROM (
                 SELECT id FROM pending_observations
                 WHERE status = 'failed'
                   AND archived_at_epoch IS NULL
                   AND project = ?1
                 ORDER BY updated_at_epoch DESC
                 LIMIT ?2
             )",
            params![project, limit],
            |row| row.get(0),
        )?
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM (
                 SELECT id FROM pending_observations
                 WHERE status = 'failed'
                   AND archived_at_epoch IS NULL
                 ORDER BY updated_at_epoch DESC
                 LIMIT ?1
             )",
            params![limit],
            |row| row.get(0),
        )?
    };
    Ok(count.max(0) as usize)
}

pub fn count_failed_purge_candidates(
    conn: &Connection,
    project: Option<&str>,
    older_than_days: i64,
) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - older_than_days.max(0) * 86_400;
    let count: i64 = if let Some(project) = project {
        conn.query_row(
            "SELECT COUNT(*) FROM pending_observations
             WHERE status = 'failed' AND project = ?1 AND updated_at_epoch < ?2",
            params![project, cutoff],
            |row| row.get(0),
        )?
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM pending_observations
             WHERE status = 'failed' AND updated_at_epoch < ?1",
            params![cutoff],
            |row| row.get(0),
        )?
    };
    Ok(count.max(0) as usize)
}
