use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureDropInput<'a> {
    pub host: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub project: Option<&'a str>,
    pub tool_name: Option<&'a str>,
    pub reason: &'a str,
    pub detail: Option<&'a str>,
    pub spill_path: Option<&'a str>,
    pub recovered_event_id: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureDropStats {
    pub total: i64,
    pub actionable: i64,
    pub unrecovered_spills: i64,
    pub latest_epoch: Option<i64>,
    pub latest_reason: Option<String>,
    pub latest_detail: Option<String>,
}

pub fn record_capture_drop(conn: &Connection, input: &CaptureDropInput<'_>) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let detail = input.detail.map(crate::db::capture::redact_capture_content);
    conn.execute(
        "INSERT INTO capture_drop_events
         (host_id, session_id, project, tool_name, reason, detail, spill_path,
          recovered_event_id, created_at_epoch, recovered_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, CASE WHEN ?8 IS NULL THEN NULL ELSE ?9 END)",
        params![
            input.host,
            input.session_id,
            input.project,
            input.tool_name,
            input.reason,
            detail,
            input.spill_path,
            input.recovered_event_id,
            now,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn query_capture_drop_stats(conn: &Connection) -> Result<CaptureDropStats> {
    if !capture_drop_table_exists(conn)? {
        return Ok(CaptureDropStats::default());
    }

    let total = conn.query_row("SELECT COUNT(*) FROM capture_drop_events", [], |row| {
        row.get(0)
    })?;
    let actionable = conn.query_row(
        "SELECT COUNT(*)
         FROM capture_drop_events
         WHERE reason NOT IN ('adapter_skip', 'codex_bash_disabled', 'bash_read_only')
           AND NOT (
               reason IN ('db_open_failed', 'capture_persistence_failed')
               AND recovered_event_id IS NOT NULL
           )",
        [],
        |row| row.get(0),
    )?;
    let unrecovered_spills = conn.query_row(
        "SELECT COUNT(*)
         FROM capture_drop_events
         WHERE reason IN ('db_open_failed', 'capture_persistence_failed')
           AND recovered_event_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    let latest = conn
        .query_row(
            "SELECT created_at_epoch, reason, detail
             FROM capture_drop_events
             ORDER BY created_at_epoch DESC, id DESC
             LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?;

    Ok(match latest {
        Some((latest_epoch, latest_reason, latest_detail)) => CaptureDropStats {
            total,
            actionable,
            unrecovered_spills,
            latest_epoch: Some(latest_epoch),
            latest_reason: Some(latest_reason),
            latest_detail,
        },
        None => CaptureDropStats {
            total,
            actionable,
            unrecovered_spills,
            ..CaptureDropStats::default()
        },
    })
}

fn capture_drop_table_exists(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM sqlite_master
         WHERE type = 'table' AND name = ?1",
        ["capture_drop_events"],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{query_capture_drop_stats, record_capture_drop, CaptureDropInput};

    #[test]
    fn capture_drop_stats_default_when_table_missing() -> anyhow::Result<()> {
        let conn = Connection::open_in_memory()?;

        let stats = query_capture_drop_stats(&conn)?;

        assert_eq!(stats.total, 0);
        assert_eq!(stats.actionable, 0);
        assert_eq!(stats.unrecovered_spills, 0);
        assert_eq!(stats.latest_reason, None);
        Ok(())
    }

    #[test]
    fn capture_drop_stats_report_latest_and_unrecovered_spills() -> anyhow::Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("CREATE TABLE captured_events (id INTEGER PRIMARY KEY);")?;
        conn.execute_batch(include_str!("../migrations/v036_capture_drop_events.sql"))?;

        record_capture_drop(
            &conn,
            &CaptureDropInput {
                host: Some("codex-cli"),
                session_id: Some("session-a"),
                project: Some("/repo"),
                tool_name: Some("Edit"),
                reason: "db_open_failed",
                detail: Some("database is locked"),
                spill_path: Some("/tmp/spill.jsonl"),
                recovered_event_id: None,
            },
        )?;
        record_capture_drop(
            &conn,
            &CaptureDropInput {
                host: Some("codex-cli"),
                session_id: Some("session-b"),
                project: Some("/repo"),
                tool_name: Some("Read"),
                reason: "adapter_skip",
                detail: Some("read-only tool"),
                spill_path: None,
                recovered_event_id: None,
            },
        )?;

        let stats = query_capture_drop_stats(&conn)?;

        assert_eq!(stats.total, 2);
        assert_eq!(stats.actionable, 1);
        assert_eq!(stats.unrecovered_spills, 1);
        assert_eq!(stats.latest_reason.as_deref(), Some("adapter_skip"));
        assert_eq!(stats.latest_detail.as_deref(), Some("read-only tool"));
        Ok(())
    }

    #[test]
    fn capture_drop_stats_treat_recovered_persistence_spills_as_non_actionable(
    ) -> anyhow::Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("CREATE TABLE captured_events (id INTEGER PRIMARY KEY);")?;
        conn.execute_batch(include_str!("../migrations/v036_capture_drop_events.sql"))?;
        conn.execute("INSERT INTO captured_events (id) VALUES (42)", [])?;

        record_capture_drop(
            &conn,
            &CaptureDropInput {
                host: Some("codex-cli"),
                session_id: Some("session-recovered"),
                project: Some("/repo"),
                tool_name: Some("Edit"),
                reason: "capture_persistence_failed",
                detail: Some("events insert failed"),
                spill_path: Some("/tmp/spill.jsonl"),
                recovered_event_id: Some(42),
            },
        )?;
        record_capture_drop(
            &conn,
            &CaptureDropInput {
                host: Some("codex-cli"),
                session_id: Some("session-open"),
                project: Some("/repo"),
                tool_name: Some("Edit"),
                reason: "capture_persistence_failed",
                detail: Some("events still blocked"),
                spill_path: Some("/tmp/spill.jsonl"),
                recovered_event_id: None,
            },
        )?;

        let stats = query_capture_drop_stats(&conn)?;

        assert_eq!(stats.total, 2);
        assert_eq!(stats.actionable, 1);
        assert_eq!(stats.unrecovered_spills, 1);
        Ok(())
    }

    #[test]
    fn capture_drop_redacts_sensitive_detail() -> anyhow::Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("CREATE TABLE captured_events (id INTEGER PRIMARY KEY);")?;
        conn.execute_batch(include_str!("../migrations/v036_capture_drop_events.sql"))?;

        record_capture_drop(
            &conn,
            &CaptureDropInput {
                host: Some("codex-cli"),
                session_id: Some("session-secret"),
                project: Some("/repo"),
                tool_name: Some("Bash"),
                reason: "codex_bash_disabled",
                detail: Some(
                    "curl -H 'Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz123456'",
                ),
                spill_path: None,
                recovered_event_id: None,
            },
        )?;

        let detail: String = conn.query_row(
            "SELECT detail FROM capture_drop_events WHERE session_id = 'session-secret'",
            [],
            |row| row.get(0),
        )?;
        assert!(detail.contains("[REDACTED]"));
        assert!(!detail.contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));
        Ok(())
    }
}
