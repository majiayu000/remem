use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use super::SECONDS_PER_DAY;

pub(super) fn count_archived_rows(
    conn: &Connection,
    table: &str,
    failed_predicate: &str,
    cutoff_epoch: i64,
) -> Result<usize> {
    let sql = format!(
        "SELECT COUNT(*) FROM {table}
         WHERE {failed_predicate}
           AND archived_at_epoch IS NOT NULL
           AND archived_at_epoch < ?1"
    );
    let count: i64 = conn.query_row(&sql, [cutoff_epoch], |row| row.get(0))?;
    Ok(count.max(0) as usize)
}

pub(super) fn count_purgeable_extraction_tasks(
    conn: &Connection,
    cutoff_epoch: i64,
) -> Result<usize> {
    Ok(purgeable_extraction_task_ids(conn, cutoff_epoch)?.len())
}

pub(super) fn archived_replay_range_ids(conn: &Connection, cutoff_epoch: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT id
         FROM extraction_replay_ranges
         WHERE status IN ('pending', 'failed', 'quarantined')
           AND archived_at_epoch IS NOT NULL
           AND archived_at_epoch < ?1
         ORDER BY archived_at_epoch ASC, id ASC",
    )?;
    let rows = stmt.query_map([cutoff_epoch], |row| row.get::<_, i64>(0))?;
    collect_i64_rows(rows)
}

pub(super) fn purgeable_extraction_task_ids(
    conn: &Connection,
    cutoff_epoch: i64,
) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT t.id
         FROM extraction_tasks t
         WHERE t.status = 'failed'
           AND t.archived_at_epoch IS NOT NULL
           AND t.archived_at_epoch < ?1
           AND NOT EXISTS (
             SELECT 1 FROM extraction_replay_ranges r
             WHERE (r.source_task_id = t.id OR r.replay_task_id = t.id)
                AND NOT (
                    r.status IN ('pending', 'failed', 'quarantined')
                    AND r.archived_at_epoch IS NOT NULL
                    AND r.archived_at_epoch < ?1
                )
           )
         ORDER BY t.archived_at_epoch ASC, t.id ASC",
    )?;
    let rows = stmt.query_map([cutoff_epoch], |row| row.get::<_, i64>(0))?;
    collect_i64_rows(rows)
}

fn collect_i64_rows<F>(rows: rusqlite::MappedRows<'_, F>) -> Result<Vec<i64>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<i64>,
{
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
}

pub(super) fn id_placeholders(len: usize, start_idx: usize) -> String {
    (start_idx..start_idx + len)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn cutoff_epoch(now_epoch: i64, days: i64) -> i64 {
    now_epoch.saturating_sub(days.max(0).saturating_mul(SECONDS_PER_DAY))
}

pub(super) fn failure_columns_available(conn: &Connection) -> Result<bool> {
    Ok(column_exists(conn, "jobs", "archived_at_epoch")?
        && table_exists(conn, "failure_lifecycle_daily")?)
}

pub(super) fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

pub(super) fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    if !table_exists(conn, table)? {
        return Ok(false);
    }
    let sql = format!("PRAGMA table_info({})", quote_identifier(table));
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
