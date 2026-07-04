use anyhow::Result;
use rusqlite::{params, Connection};

use super::sql::{column_exists, table_exists};
use super::{FailureSurfaceStats, MAX_FAILURE_AUTO_RETRIES, SECONDS_PER_DAY};

#[derive(Debug, Clone, Copy)]
pub(super) struct SurfaceQuery {
    pub(super) surface: &'static str,
    pub(super) table: &'static str,
    pub(super) failed_predicate: &'static str,
    pub(super) attempt_column: &'static str,
    pub(super) created_column: &'static str,
    pub(super) updated_column: &'static str,
}

pub(super) fn query_surface_stats(
    conn: &Connection,
    query: SurfaceQuery,
    now_epoch: i64,
) -> Result<FailureSurfaceStats> {
    if !table_exists(conn, query.table)? {
        return Ok(FailureSurfaceStats::default());
    }
    let has_lifecycle = column_exists(conn, query.table, "archived_at_epoch")?;
    let attempt_expr = if column_exists(conn, query.table, query.attempt_column)? {
        query.attempt_column
    } else {
        "0"
    };
    let updated_expr = if column_exists(conn, query.table, query.updated_column)? {
        query.updated_column
    } else {
        "0"
    };
    let created_expr = if column_exists(conn, query.table, query.created_column)? {
        query.created_column
    } else {
        "0"
    };
    let class_expr = if has_lifecycle {
        "COALESCE(failure_class, 'transient')".to_string()
    } else {
        "'transient'".to_string()
    };
    let failed_at_expr = if has_lifecycle {
        format!(
            "COALESCE(failed_at_epoch, NULLIF({updated}, 0), {created})",
            updated = updated_expr,
            created = created_expr
        )
    } else {
        format!(
            "COALESCE(NULLIF({updated}, 0), {created})",
            updated = updated_expr,
            created = created_expr
        )
    };
    let archived_filter = if has_lifecycle {
        "archived_at_epoch IS NULL"
    } else {
        "1 = 1"
    };
    let week_ago = now_epoch.saturating_sub(7 * SECONDS_PER_DAY);
    let sql = format!(
        "SELECT
            COUNT(*) AS actionable_total,
            COALESCE(SUM(CASE WHEN {failed_at_expr} >= ?1 THEN 1 ELSE 0 END), 0) AS actionable_7d,
            COALESCE(SUM(CASE WHEN {class_expr} = 'transient' THEN 1 ELSE 0 END), 0) AS transient,
            COALESCE(SUM(CASE WHEN {class_expr} = 'permanent' THEN 1 ELSE 0 END), 0) AS permanent,
            COALESCE(SUM(CASE WHEN {attempt_col} >= ?2 THEN 1 ELSE 0 END), 0) AS exhausted,
            MIN({failed_at_expr}) AS oldest_actionable_epoch
         FROM {table}
         WHERE {failed_predicate}
           AND {archived_filter}",
        attempt_col = attempt_expr,
        table = query.table,
        failed_predicate = query.failed_predicate
    );
    let (actionable_total, actionable_7d, transient, permanent, exhausted, oldest): (
        i64,
        i64,
        i64,
        i64,
        i64,
        Option<i64>,
    ) = conn.query_row(&sql, params![week_ago, MAX_FAILURE_AUTO_RETRIES], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
        ))
    })?;
    let archived = if has_lifecycle {
        let sql = format!(
            "SELECT COUNT(*) FROM {table}
             WHERE {failed_predicate} AND archived_at_epoch IS NOT NULL",
            table = query.table,
            failed_predicate = query.failed_predicate
        );
        conn.query_row(&sql, [], |row| row.get(0))?
    } else {
        0
    };
    let (historical_archived, historical_purged) = query_historical_counts(conn, query.surface)?;
    Ok(FailureSurfaceStats {
        actionable_total,
        actionable_7d,
        transient,
        permanent,
        exhausted,
        archived,
        historical_archived,
        historical_purged,
        oldest_actionable_epoch: oldest,
    })
}

fn query_historical_counts(conn: &Connection, surface: &str) -> Result<(i64, i64)> {
    if !table_exists(conn, "failure_lifecycle_daily")? {
        return Ok((0, 0));
    }
    conn.query_row(
        "SELECT COALESCE(SUM(archived_count), 0), COALESCE(SUM(purged_count), 0)
         FROM failure_lifecycle_daily
         WHERE surface = ?1",
        [surface],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(Into::into)
}
