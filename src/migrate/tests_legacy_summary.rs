use anyhow::Result;
use rusqlite::{params, Connection};

use super::run_migrations;

fn insert_job(conn: &Connection, id: i64, job_type: &str, state: &str) -> Result<()> {
    let now = 1_700_000_000_i64 + id;
    conn.execute(
        "INSERT INTO jobs
         (id, host, job_type, project, session_id, payload_json, state, priority,
          attempt_count, max_attempts, lease_owner, lease_expires_epoch,
          next_retry_epoch, last_error, created_at_epoch, updated_at_epoch,
          failure_class, failed_at_epoch, archived_at_epoch)
         VALUES (?1, 'codex-cli', ?2, '/repo', 'session-a', '{}', ?3, 100,
          1, 6, NULL, NULL, 99, NULL, ?4, ?4, NULL, NULL, NULL)",
        params![id, job_type, state, now],
    )?;
    Ok(())
}

#[test]
fn legacy_summary_upgrade_rejects_non_terminal_jobs() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    conn.execute("DELETE FROM _schema_migrations WHERE version = 64", [])?;
    insert_job(&conn, 1, "summary", "pending")?;
    insert_job(&conn, 2, "summary", "processing")?;
    insert_job(&conn, 3, "summary", "done")?;
    insert_job(&conn, 4, "summary", "failed")?;
    insert_job(&conn, 5, "compress", "pending")?;
    conn.execute(
        "UPDATE jobs
         SET lease_owner = 'worker-a',
             lease_expires_epoch = 1700001000,
             attempt_count = 2
         WHERE id = 2",
        [],
    )?;
    conn.execute(
        "UPDATE jobs
         SET last_error = 'old failure',
             failure_class = 'transient',
             failed_at_epoch = 44,
             attempt_count = 3
         WHERE id = 4",
        [],
    )?;

    run_migrations(&conn)?;

    for id in [1_i64, 2] {
        let row: (
            String,
            i64,
            i64,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<i64>,
            Option<i64>,
        ) = conn.query_row(
            "SELECT state, attempt_count, next_retry_epoch, last_error,
                    failure_class, failed_at_epoch, lease_owner,
                    lease_expires_epoch, archived_at_epoch
             FROM jobs WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )?;
        assert_eq!(row.0, "failed");
        assert_eq!(row.1, 6);
        assert_eq!(row.2, 0);
        assert!(
            row.3
                .as_deref()
                .unwrap_or_default()
                .contains("legacy summary job rejected"),
            "summary job {id} should record the upgrade rejection"
        );
        assert_eq!(row.4.as_deref(), Some("permanent"));
        assert!(row.5.is_some(), "summary job {id} should record failed_at");
        assert_eq!(row.6, None);
        assert_eq!(row.7, None);
        assert_eq!(row.8, None);
    }

    let done_state: String =
        conn.query_row("SELECT state FROM jobs WHERE id = 3", [], |row| row.get(0))?;
    assert_eq!(done_state, "done");

    let failed_row: (String, i64, Option<String>, Option<String>, Option<i64>) = conn.query_row(
        "SELECT state, attempt_count, last_error, failure_class, failed_at_epoch
         FROM jobs WHERE id = 4",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    assert_eq!(failed_row.0, "failed");
    assert_eq!(failed_row.1, 3);
    assert_eq!(failed_row.2.as_deref(), Some("old failure"));
    assert_eq!(failed_row.3.as_deref(), Some("transient"));
    assert_eq!(failed_row.4, Some(44));

    let compress_state: String =
        conn.query_row("SELECT state FROM jobs WHERE id = 5", [], |row| row.get(0))?;
    assert_eq!(compress_state, "pending");

    let applied: i64 = conn.query_row(
        "SELECT COUNT(*) FROM _schema_migrations
         WHERE version = 64 AND name = 'reject_legacy_summary_jobs'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(applied, 1);
    Ok(())
}
