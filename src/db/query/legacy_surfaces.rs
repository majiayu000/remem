use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacySurfaceStats {
    pub surface: String,
    pub disposition: String,
    pub row_count: i64,
    pub last_write_epoch: Option<i64>,
    pub frozen_write_violations: i64,
}

pub(super) fn query_legacy_surface_stats(conn: &Connection) -> Result<Vec<LegacySurfaceStats>> {
    let observations = legacy_table_surface(conn, "observations", "reclassify-current", &[])?;
    let observations_fts =
        legacy_table_surface(conn, "observations_fts", "reclassify-current", &[])?;
    let session_summaries = legacy_table_surface(conn, "session_summaries", "keep", &[])?;
    let pending_observations = legacy_pending_observation_surface(conn)?;
    let summary_jobs = legacy_summary_job_surface(conn)?;

    Ok(vec![
        observations,
        observations_fts,
        session_summaries,
        pending_observations,
        summary_jobs,
    ])
}

fn legacy_table_surface(
    conn: &Connection,
    table: &str,
    disposition: &str,
    violation_epoch_columns: &[&str],
) -> Result<LegacySurfaceStats> {
    let row_count = table_count_or_zero(conn, table)?;
    let last_write_epoch = max_write_epoch(conn, table)?;
    let mut has_violation_epoch = false;
    for column in violation_epoch_columns {
        has_violation_epoch |= column_exists(conn, table, column)?;
    }
    let frozen_write_violations = if row_count > 0 && has_violation_epoch {
        row_count
    } else {
        0
    };
    Ok(LegacySurfaceStats {
        surface: table.to_string(),
        disposition: disposition.to_string(),
        row_count,
        last_write_epoch,
        frozen_write_violations,
    })
}

fn legacy_pending_observation_surface(conn: &Connection) -> Result<LegacySurfaceStats> {
    let table = "pending_observations";
    let row_count = table_count_or_zero(conn, table)?;
    let last_write_epoch = max_write_epoch(conn, table)?;
    let frozen_write_violations = if !table_exists(conn, table)? {
        0
    } else if column_exists(conn, table, "status")? {
        let archived_filter = if column_exists(conn, table, "archived_at_epoch")? {
            "AND archived_at_epoch IS NULL"
        } else {
            ""
        };
        conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM pending_observations
                 WHERE status <> 'migrated'
                 {archived_filter}"
            ),
            [],
            |row| row.get(0),
        )?
    } else {
        row_count
    };

    Ok(LegacySurfaceStats {
        surface: table.to_string(),
        disposition: "retire".to_string(),
        row_count,
        last_write_epoch,
        frozen_write_violations,
    })
}

fn legacy_summary_job_surface(conn: &Connection) -> Result<LegacySurfaceStats> {
    let surface = "summary_jobs".to_string();
    let disposition = "retire-summary-only".to_string();
    if !table_exists(conn, "jobs")? || !column_exists(conn, "jobs", "job_type")? {
        return Ok(LegacySurfaceStats {
            surface,
            disposition,
            row_count: 0,
            last_write_epoch: None,
            frozen_write_violations: 0,
        });
    }

    let row_count = conn.query_row(
        "SELECT COUNT(*) FROM jobs WHERE job_type = 'summary'",
        [],
        |row| row.get(0),
    )?;
    let frozen_write_violations = if column_exists(conn, "jobs", "state")? {
        let archived_filter = if column_exists(conn, "jobs", "archived_at_epoch")? {
            "AND archived_at_epoch IS NULL"
        } else {
            ""
        };
        conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM jobs
                 WHERE job_type = 'summary'
                   AND state <> 'done'
                   AND NOT (
                     state = 'failed'
                     AND failure_class = 'permanent'
                     AND last_error IN (
                       'legacy summary job rejected during GH684 summary retirement upgrade; SessionRollup owns session summary output',
                       'legacy Summary jobs are retired; SessionRollup owns session summary output'
                     )
                   )
                   {archived_filter}"
            ),
            [],
            |row| row.get(0),
        )?
    } else {
        row_count
    };
    let last_write_epoch = max_write_epoch_where(conn, "jobs", "job_type = 'summary'")?;
    Ok(LegacySurfaceStats {
        surface,
        disposition,
        row_count,
        last_write_epoch,
        frozen_write_violations,
    })
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn table_count_or_zero(conn: &Connection, table: &str) -> Result<i64> {
    if !table_exists(conn, table)? {
        return Ok(0);
    }
    Ok(conn.query_row(
        &format!("SELECT COUNT(*) FROM {}", quote_identifier(table)),
        [],
        |row| row.get(0),
    )?)
}

fn max_write_epoch(conn: &Connection, table: &str) -> Result<Option<i64>> {
    max_write_epoch_where(conn, table, "1 = 1")
}

fn max_write_epoch_where(
    conn: &Connection,
    table: &str,
    where_clause: &str,
) -> Result<Option<i64>> {
    if !table_exists(conn, table)? {
        return Ok(None);
    }
    let mut columns = Vec::new();
    if column_exists(conn, table, "updated_at_epoch")? {
        columns.push("updated_at_epoch");
    }
    if column_exists(conn, table, "created_at_epoch")? {
        columns.push("created_at_epoch");
    }
    if columns.is_empty() {
        return Ok(None);
    }

    let expression = match columns.as_slice() {
        ["updated_at_epoch", "created_at_epoch"] => {
            "CASE
                WHEN NULLIF(updated_at_epoch, 0) IS NULL THEN NULLIF(created_at_epoch, 0)
                WHEN NULLIF(created_at_epoch, 0) IS NULL THEN NULLIF(updated_at_epoch, 0)
                WHEN updated_at_epoch >= created_at_epoch THEN updated_at_epoch
                ELSE created_at_epoch
             END"
        }
        [single] => match *single {
            "updated_at_epoch" => "NULLIF(updated_at_epoch, 0)",
            "created_at_epoch" => "NULLIF(created_at_epoch, 0)",
            _ => unreachable!("legacy write epoch columns are fixed"),
        },
        _ => unreachable!("legacy write epoch columns are fixed"),
    };
    let sql = format!(
        "SELECT MAX({expression}) FROM {} WHERE {where_clause}",
        quote_identifier(table)
    );
    Ok(conn.query_row(&sql, [], |row| row.get(0))?)
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    if !table_exists(conn, table)? {
        return Ok(false);
    }
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", quote_identifier(table)))?;
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
