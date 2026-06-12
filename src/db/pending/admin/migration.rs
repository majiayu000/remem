use anyhow::{bail, Result};
use rusqlite::{params, Connection};
use serde::Serialize;

use crate::db::{self, CaptureEventInput, ExtractionTaskKind};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LegacyPendingMigration {
    pub pending_id: i64,
    pub event_id: String,
    pub captured_event_id: i64,
    pub extraction_task_id: i64,
    pub host: String,
    pub project: String,
    pub session_id: String,
}

struct LegacyPendingRow {
    id: i64,
    host: String,
    session_id: String,
    project: String,
    tool_name: String,
    tool_input: Option<String>,
    tool_response: Option<String>,
    cwd: Option<String>,
    created_at_epoch: i64,
}

pub fn count_legacy_migration_candidates(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<usize> {
    let limit = limit.max(1);
    let count: i64 = if let Some(project) = project {
        conn.query_row(
            "SELECT COUNT(*) FROM (
                 SELECT id FROM pending_observations
                 WHERE status = 'pending' AND project = ?1
                 ORDER BY created_at_epoch ASC, id ASC
                 LIMIT ?2
             )",
            params![project, limit],
            |row| row.get(0),
        )?
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM (
                 SELECT id FROM pending_observations
                 WHERE status = 'pending'
                 ORDER BY created_at_epoch ASC, id ASC
                 LIMIT ?1
             )",
            params![limit],
            |row| row.get(0),
        )?
    };
    Ok(count.max(0) as usize)
}

pub fn migrate_legacy_pending(
    conn: &mut Connection,
    project: Option<&str>,
    fallback_host: Option<&str>,
    limit: i64,
) -> Result<Vec<LegacyPendingMigration>> {
    let fallback_host = fallback_host.map(normalize_capture_host).transpose()?;
    let tx = conn.transaction()?;
    let rows = select_legacy_pending_rows(&tx, project, limit)?;
    let mut migrated = Vec::new();

    for row in rows {
        let host = capture_host_for_row(&row.host, fallback_host)?;
        let event_id = legacy_event_id(row.id);
        let content = legacy_capture_content(&row);
        let outcome = db::record_captured_event_with_id(
            &tx,
            &CaptureEventInput {
                host,
                session_id: &row.session_id,
                project: &row.project,
                cwd: row.cwd.as_deref(),
                event_type: "tool_result",
                role: None,
                tool_name: Some(&row.tool_name),
                content: &content,
                task_kind: Some(ExtractionTaskKind::ObservationExtract),
            },
            Some(&event_id),
        )?;
        let extraction_task_id = outcome.extraction_task_id.ok_or_else(|| {
            anyhow::anyhow!("legacy pending migration did not enqueue extraction")
        })?;
        let changed = tx.execute(
            "UPDATE pending_observations
             SET status = 'migrated',
                 lease_owner = NULL,
                 lease_expires_epoch = NULL,
                 next_retry_epoch = NULL,
                 last_error = NULL,
                 updated_at_epoch = ?2
             WHERE id = ?1 AND status = 'pending'",
            params![row.id, chrono::Utc::now().timestamp()],
        )?;
        if changed != 1 {
            bail!("legacy pending row {} changed while migrating", row.id);
        }
        migrated.push(LegacyPendingMigration {
            pending_id: row.id,
            event_id,
            captured_event_id: outcome.event_row_id,
            extraction_task_id,
            host: host.to_string(),
            project: row.project,
            session_id: row.session_id,
        });
    }

    tx.commit()?;
    Ok(migrated)
}

fn select_legacy_pending_rows(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<LegacyPendingRow>> {
    let limit = limit.max(1);
    let sql = if project.is_some() {
        "SELECT id, host, session_id, project, tool_name, tool_input, tool_response, cwd, created_at_epoch
         FROM pending_observations
         WHERE status = 'pending' AND project = ?1
         ORDER BY created_at_epoch ASC, id ASC
         LIMIT ?2"
    } else {
        "SELECT id, host, session_id, project, tool_name, tool_input, tool_response, cwd, created_at_epoch
         FROM pending_observations
         WHERE status = 'pending'
         ORDER BY created_at_epoch ASC, id ASC
         LIMIT ?1"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = if let Some(project) = project {
        stmt.query_map(params![project, limit], row_from_db)?
    } else {
        stmt.query_map(params![limit], row_from_db)?
    };
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn row_from_db(row: &rusqlite::Row<'_>) -> rusqlite::Result<LegacyPendingRow> {
    Ok(LegacyPendingRow {
        id: row.get(0)?,
        host: row.get(1)?,
        session_id: row.get(2)?,
        project: row.get(3)?,
        tool_name: row.get(4)?,
        tool_input: row.get(5)?,
        tool_response: row.get(6)?,
        cwd: row.get(7)?,
        created_at_epoch: row.get(8)?,
    })
}

fn capture_host_for_row<'a>(row_host: &'a str, fallback_host: Option<&'a str>) -> Result<&'a str> {
    match normalize_capture_host(row_host) {
        Ok(host) => Ok(host),
        Err(_) => fallback_host
            .ok_or_else(|| anyhow::anyhow!("legacy pending row has host='{row_host}'; pass --host claude-code or --host codex-cli")),
    }
}

fn normalize_capture_host(host: &str) -> Result<&str> {
    match host {
        crate::runtime_config::CLAUDE_HOST | crate::runtime_config::CODEX_HOST => Ok(host),
        _ => bail!("invalid capture host '{host}'"),
    }
}

fn legacy_event_id(id: i64) -> String {
    format!("legacy-pending-{id}")
}

fn legacy_capture_content(row: &LegacyPendingRow) -> String {
    let git_branch = row.cwd.as_deref().and_then(db::detect_git_branch);
    serde_json::json!({
        "summary": format!("Recovered legacy {} event", row.tool_name),
        "event_type": "legacy_pending_observation",
        "detail": format!(
            "Recovered from pending_observations id={} created_at_epoch={}",
            row.id, row.created_at_epoch
        ),
        "files": serde_json::Value::Null,
        "exit_code": serde_json::Value::Null,
        "tool_name": row.tool_name,
        "tool_input": parse_jsonish(row.tool_input.as_deref()),
        "tool_response": parse_jsonish(row.tool_response.as_deref()),
        "git_branch": git_branch,
    })
    .to_string()
}

fn parse_jsonish(value: Option<&str>) -> serde_json::Value {
    match value {
        Some(value) => serde_json::from_str(value)
            .unwrap_or_else(|_| serde_json::Value::String(value.to_string())),
        None => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_host_count_is_queryable() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE pending_observations (
                id INTEGER PRIMARY KEY,
                host TEXT NOT NULL,
                session_id TEXT NOT NULL,
                project TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                tool_input TEXT,
                tool_response TEXT,
                cwd TEXT,
                created_at_epoch INTEGER NOT NULL,
                updated_at_epoch INTEGER NOT NULL,
                status TEXT NOT NULL,
                attempt_count INTEGER NOT NULL,
                next_retry_epoch INTEGER,
                last_error TEXT,
                lease_owner TEXT,
                lease_expires_epoch INTEGER
            );",
        )?;
        conn.execute(
            "INSERT INTO pending_observations
             (host, session_id, project, tool_name, created_at_epoch, updated_at_epoch, status, attempt_count)
             VALUES ('unknown', 's', 'p', 'Edit', 1, 1, 'pending', 0)",
            [],
        )?;

        assert_eq!(count_legacy_migration_candidates(&conn, Some("p"), 10)?, 1);
        assert_eq!(
            count_legacy_migration_candidates(&conn, Some("other"), 10)?,
            0
        );
        Ok(())
    }

    #[test]
    fn legacy_event_id_is_stable() {
        assert_eq!(legacy_event_id(42), "legacy-pending-42");
    }
}
