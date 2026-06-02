use anyhow::{Context, Result};
use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;

use crate::db;

pub(in crate::cli) fn run_context_gate_status(
    project: Option<&str>,
    session: Option<&str>,
    limit: i64,
    json: bool,
) -> Result<()> {
    let conn = open_context_gate_db_read_only()?;
    let project = project.map(db::project_from_cwd);
    let limit = limit.clamp(1, 200);
    let rows = load_recent_context_gate_rows(&conn, project.as_deref(), session, limit)?;
    let report = ContextGateStatusReport {
        database: db::db_path().display().to_string(),
        filters: ContextGateStatusFilters {
            project,
            session: session.map(str::to_string),
            limit,
        },
        rows,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_context_gate_status(&report);
    }
    Ok(())
}

fn open_context_gate_db_read_only() -> Result<Connection> {
    let db_path = db::db_path();
    if !db_path.exists() {
        anyhow::bail!("remem database not found at {}", db_path.display());
    }
    let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open read-only remem database {}", db_path.display()))?;
    db::apply_cipher_key_if_available(&conn)
        .with_context(|| format!("unlock read-only remem database {}", db_path.display()))?;
    Ok(conn)
}

fn load_recent_context_gate_rows(
    conn: &Connection,
    project: Option<&str>,
    session: Option<&str>,
    limit: i64,
) -> Result<Vec<ContextGateStatusRow>> {
    if !context_injections_table_exists(conn)? {
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT host, project, injection_key, session_id, hook_source, output_mode,
                output_chars, updated_at_epoch, last_emitted_epoch, emit_count, suppress_count
         FROM context_injections
         WHERE (?1 IS NULL OR project = ?1)
           AND (?2 IS NULL OR session_id = ?2)
         ORDER BY updated_at_epoch DESC
         LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![project, session, limit], |row| {
            let mut status = ContextGateStatusRow {
                host: row.get(0)?,
                project: row.get(1)?,
                injection_key: row.get(2)?,
                session_id: row.get(3)?,
                hook_source: row.get(4)?,
                output_mode: row.get(5)?,
                output_chars: row.get(6)?,
                updated_at_epoch: row.get(7)?,
                updated_at: String::new(),
                last_emitted_epoch: row.get(8)?,
                last_emitted_at: String::new(),
                emit_count: row.get(9)?,
                suppress_count: row.get(10)?,
                reason: String::new(),
            };
            status.updated_at = format_context_gate_timestamp(status.updated_at_epoch);
            status.last_emitted_at = format_context_gate_timestamp(status.last_emitted_epoch);
            status.reason = infer_context_gate_reason(&status).to_string();
            Ok(status)
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn context_injections_table_exists(conn: &Connection) -> Result<bool> {
    let exists = conn.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'context_injections'
         )",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(exists != 0)
}

fn print_context_gate_status(report: &ContextGateStatusReport) {
    println!("Recent context injections:");
    if report.rows.is_empty() {
        println!("  (none)");
        return;
    }

    for row in &report.rows {
        println!(
            "  {} {} {} source={} reason={} project={} session={} emits={} suppressions={}",
            row.updated_at,
            row.host,
            row.output_mode,
            row.hook_source.as_deref().unwrap_or("-"),
            row.reason,
            row.project,
            row.session_id.as_deref().unwrap_or("-"),
            row.emit_count,
            row.suppress_count
        );
    }
}

fn infer_context_gate_reason(row: &ContextGateStatusRow) -> &'static str {
    match row.output_mode.as_str() {
        "delta" => "changed_hash",
        "suppressed" if source_is_default_suppressed(row.hook_source.as_deref()) => {
            "suppressed_source"
        }
        "suppressed" => "same_hash_or_strict",
        "full" if row.emit_count <= 1 => "first_or_forced",
        "full" if source_requires_restart(row.hook_source.as_deref()) => "restart_source",
        "full" => "repeat_full_or_forced",
        _ => "unknown",
    }
}

fn source_is_default_suppressed(source: Option<&str>) -> bool {
    matches!(
        source.map(|value| value.trim().to_ascii_lowercase()),
        Some(value) if value == "compact"
    )
}

fn source_requires_restart(source: Option<&str>) -> bool {
    matches!(
        source.map(|value| value.trim().to_ascii_lowercase()),
        Some(value) if value == "clear"
    )
}

fn format_context_gate_timestamp(epoch: i64) -> String {
    chrono::DateTime::from_timestamp(epoch, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_default()
}

#[derive(Debug, Serialize)]
struct ContextGateStatusReport {
    database: String,
    filters: ContextGateStatusFilters,
    rows: Vec<ContextGateStatusRow>,
}

#[derive(Debug, Serialize)]
struct ContextGateStatusFilters {
    project: Option<String>,
    session: Option<String>,
    limit: i64,
}

#[derive(Debug, Serialize)]
struct ContextGateStatusRow {
    host: String,
    project: String,
    injection_key: String,
    session_id: Option<String>,
    hook_source: Option<String>,
    output_mode: String,
    output_chars: i64,
    updated_at_epoch: i64,
    updated_at: String,
    last_emitted_epoch: i64,
    last_emitted_at: String,
    emit_count: i64,
    suppress_count: i64,
    reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_context_gate_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!(
            "../../migrations/v016_context_injection_gate.sql"
        ))
        .unwrap();
        conn
    }

    #[test]
    fn context_gate_status_reads_recent_rows() -> Result<()> {
        let conn = setup_context_gate_conn();
        conn.execute(
            "INSERT INTO context_injections
             (host, project, injection_key, session_id, transcript_path, hook_source,
              context_hash, output_mode, output_chars, created_at_epoch, updated_at_epoch,
              last_emitted_epoch, emit_count, suppress_count)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                "codex-cli",
                "/tmp/remem",
                "session:/tmp/remem:sess-1",
                "sess-1",
                "compact",
                "hash-a",
                "suppressed",
                0,
                100,
                110,
                100,
                1,
                2,
            ],
        )?;

        let rows = load_recent_context_gate_rows(&conn, Some("/tmp/remem"), Some("sess-1"), 20)?;

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].host, "codex-cli");
        assert_eq!(rows[0].hook_source.as_deref(), Some("compact"));
        assert_eq!(rows[0].output_mode, "suppressed");
        assert_eq!(rows[0].reason, "suppressed_source");
        assert_eq!(rows[0].emit_count, 1);
        assert_eq!(rows[0].suppress_count, 2);
        Ok(())
    }

    #[test]
    fn context_gate_status_missing_table_returns_no_rows() -> Result<()> {
        let conn = Connection::open_in_memory()?;

        let rows = load_recent_context_gate_rows(&conn, None, None, 20)?;

        assert!(rows.is_empty());
        Ok(())
    }
}
