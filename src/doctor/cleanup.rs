use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension};

use super::types::{Check, Status};

const CHECK_NAME: &str = "Automatic cleanup";
const CLEANUP_COOLDOWN_SECS: i64 = 24 * 60 * 60;
const CLEANUP_GRACE_SECS: i64 = 6 * 60 * 60;
const CLEANUP_OVERDUE_SECS: i64 = CLEANUP_COOLDOWN_SECS + CLEANUP_GRACE_SECS;
const CLEANUP_STALLED_SECS: i64 = 6 * 60 * 60;
const MAX_FAILURE_ERROR_BYTES: usize = 160;

#[derive(Debug)]
struct CleanupRun {
    id: i64,
    finished_at_epoch: i64,
    error: Option<String>,
}

impl CleanupRun {
    fn recency(&self) -> (i64, i64) {
        (self.finished_at_epoch, self.id)
    }
}

#[derive(Debug)]
struct CleanupStatus {
    last_success: Option<CleanupRun>,
    last_failure: Option<CleanupRun>,
    scheduled: i64,
    processing: i64,
    stalled_scheduled: i64,
    expired_processing: i64,
}

pub(super) fn check_cleanup_status(conn: Option<&Connection>) -> Check {
    check_cleanup_status_at(conn, Utc::now().timestamp())
}

fn check_cleanup_status_at(conn: Option<&Connection>, now_epoch: i64) -> Check {
    let Some(conn) = conn else {
        return Check::new(
            CHECK_NAME,
            Status::Ok,
            "unavailable: database is not available; cleanup status was not evaluated",
        );
    };
    let status = match load_cleanup_status(conn, now_epoch) {
        Ok(Some(status)) => status,
        Ok(None) => {
            return Check::new(
                CHECK_NAME,
                Status::Ok,
                "unavailable: cleanup maintenance history is not available on this schema",
            );
        }
        Err(_) => {
            return Check::new(
                CHECK_NAME,
                Status::Warn,
                "unavailable: cleanup status query failed; inspect the schema check",
            );
        }
    };
    evaluate_cleanup_status(status, now_epoch)
}

fn load_cleanup_status(
    conn: &Connection,
    now_epoch: i64,
) -> rusqlite::Result<Option<CleanupStatus>> {
    if !table_exists(conn, "maintenance_runs")? || !table_exists(conn, "jobs")? {
        return Ok(None);
    }
    let last_success = latest_automatic_run(conn, "success")?;
    let last_failure = latest_automatic_run(conn, "failure")?;
    let stalled_before_epoch = now_epoch.saturating_sub(CLEANUP_STALLED_SECS);
    let (scheduled, processing, stalled_scheduled, expired_processing) = conn.query_row(
        "SELECT
             COALESCE(SUM(CASE WHEN state = 'pending' THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN state = 'processing' THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(
                 CASE WHEN state = 'pending'
                            AND next_retry_epoch <= ?1
                            AND updated_at_epoch < ?2
                      THEN 1 ELSE 0 END
             ), 0),
             COALESCE(SUM(
                 CASE WHEN state = 'processing'
                            AND (lease_expires_epoch IS NULL OR lease_expires_epoch < ?1)
                      THEN 1 ELSE 0 END
             ), 0)
         FROM jobs
         WHERE job_type = 'cleanup'
           AND state IN ('pending', 'processing')",
        [now_epoch, stalled_before_epoch],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    Ok(Some(CleanupStatus {
        last_success,
        last_failure,
        scheduled,
        processing,
        stalled_scheduled,
        expired_processing,
    }))
}

fn table_exists(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    let exists: i64 = conn.query_row(
        "SELECT EXISTS(
             SELECT 1
             FROM sqlite_master
             WHERE type = 'table' AND name = ?1
         )",
        [table],
        |row| row.get(0),
    )?;
    Ok(exists != 0)
}

fn latest_automatic_run(conn: &Connection, outcome: &str) -> rusqlite::Result<Option<CleanupRun>> {
    conn.query_row(
        "SELECT id, COALESCE(finished_at_epoch, started_at_epoch), error
         FROM maintenance_runs
         WHERE \"trigger\" = 'automatic'
           AND outcome = ?1
         ORDER BY COALESCE(finished_at_epoch, started_at_epoch) DESC, id DESC
         LIMIT 1",
        [outcome],
        |row| {
            Ok(CleanupRun {
                id: row.get(0)?,
                finished_at_epoch: row.get(1)?,
                error: row.get(2)?,
            })
        },
    )
    .optional()
}

fn evaluate_cleanup_status(status: CleanupStatus, now_epoch: i64) -> Check {
    let active = active_detail(&status);
    if status.stalled_scheduled > 0 || status.expired_processing > 0 {
        return Check::new(
            CHECK_NAME,
            Status::Warn,
            format!(
                "stalled: ready_pending_older_than_{}s={} expired_processing={}; {active}; last_success={}; last_failure={}",
                CLEANUP_STALLED_SECS,
                status.stalled_scheduled,
                status.expired_processing,
                optional_run_time(status.last_success.as_ref()),
                optional_run_time(status.last_failure.as_ref()),
            ),
        );
    }

    if status.scheduled > 0 || status.processing > 0 {
        return Check::new(
            CHECK_NAME,
            Status::Ok,
            format!(
                "active: {active}; last_success={}; last_failure={} (retrying if newer)",
                optional_run_time(status.last_success.as_ref()),
                optional_run_time(status.last_failure.as_ref()),
            ),
        );
    }

    let latest_failure = status.last_failure.as_ref().filter(|failure| {
        status
            .last_success
            .as_ref()
            .is_none_or(|success| failure.recency() > success.recency())
    });
    if let Some(failure) = latest_failure {
        return Check::new(
            CHECK_NAME,
            Status::Warn,
            format!(
                "failed: last_failure={}; error={}; last_success={}; {active}",
                format_run_time(failure.finished_at_epoch),
                safe_failure_error(failure.error.as_deref()),
                optional_run_time(status.last_success.as_ref()),
            ),
        );
    }

    let Some(success) = status.last_success.as_ref() else {
        return Check::new(
            CHECK_NAME,
            Status::Ok,
            format!("never_run: no automatic cleanup history; {active}"),
        );
    };
    let success_age = now_epoch.saturating_sub(success.finished_at_epoch).max(0);
    let failure_detail = status
        .last_failure
        .as_ref()
        .map(|failure| {
            format!(
                "{} (superseded)",
                format_run_time(failure.finished_at_epoch)
            )
        })
        .unwrap_or_else(|| "never".to_string());
    if success_age > CLEANUP_OVERDUE_SECS {
        return Check::new(
            CHECK_NAME,
            Status::Warn,
            format!(
                "overdue: last_success={} age={}s exceeds {}s; last_failure={failure_detail}; {active}",
                format_run_time(success.finished_at_epoch),
                success_age,
                CLEANUP_OVERDUE_SECS,
            ),
        );
    }

    Check::new(
        CHECK_NAME,
        Status::Ok,
        format!(
            "healthy: last_success={} age={}s; last_failure={failure_detail}; {active}",
            format_run_time(success.finished_at_epoch),
            success_age,
        ),
    )
}

fn active_detail(status: &CleanupStatus) -> String {
    format!(
        "scheduled={} processing={}",
        status.scheduled, status.processing
    )
}

fn optional_run_time(run: Option<&CleanupRun>) -> String {
    run.map(|run| format_run_time(run.finished_at_epoch))
        .unwrap_or_else(|| "never".to_string())
}

fn format_run_time(epoch: i64) -> String {
    DateTime::<Utc>::from_timestamp(epoch, 0)
        .map(|time| time.to_rfc3339_opts(SecondsFormat::Secs, true))
        .unwrap_or_else(|| format!("epoch:{epoch}"))
}

fn safe_failure_error(error: Option<&str>) -> String {
    let Some(error) = error.map(str::trim).filter(|error| !error.is_empty()) else {
        return "unspecified".to_string();
    };
    let scan = crate::db::truncate_str(error, MAX_FAILURE_ERROR_BYTES.saturating_add(64));
    let lower = scan.to_ascii_lowercase();
    if scan.starts_with('{')
        || scan.starts_with('[')
        || lower.contains("payload")
        || lower.contains("counts_json")
    {
        return "[structured detail redacted]".to_string();
    }
    let redacted =
        crate::adapter::common::redact_hook_payload_preview(error, MAX_FAILURE_ERROR_BYTES);
    let one_line = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.is_empty() {
        return "unspecified".to_string();
    }
    let preview = crate::db::truncate_str(&one_line, MAX_FAILURE_ERROR_BYTES);
    if error.len() > MAX_FAILURE_ERROR_BYTES || preview.len() < one_line.len() {
        format!("{preview}...")
    } else {
        preview.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 2_000_000_000;

    fn create_schema(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            "CREATE TABLE maintenance_runs (
                 id INTEGER PRIMARY KEY,
                 job_id INTEGER,
                 \"trigger\" TEXT NOT NULL,
                 policy_version INTEGER NOT NULL,
                 started_at_epoch INTEGER NOT NULL,
                 finished_at_epoch INTEGER,
                 outcome TEXT NOT NULL,
                 counts_json TEXT,
                 error TEXT
             );
             CREATE TABLE jobs (
                 id INTEGER PRIMARY KEY,
                 job_type TEXT NOT NULL,
                 state TEXT NOT NULL,
                 payload_json TEXT NOT NULL DEFAULT '{}',
                 next_retry_epoch INTEGER NOT NULL DEFAULT 0,
                 lease_expires_epoch INTEGER,
                 updated_at_epoch INTEGER NOT NULL DEFAULT 0
             );",
        )
    }

    fn insert_run(
        conn: &Connection,
        trigger: &str,
        outcome: &str,
        finished_at_epoch: i64,
        error: Option<&str>,
    ) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO maintenance_runs
                 (job_id, \"trigger\", policy_version, started_at_epoch,
                  finished_at_epoch, outcome, counts_json, error)
             VALUES (NULL, ?1, 1, ?2, ?2, ?3, '{\"deleted\":1}', ?4)",
            rusqlite::params![trigger, finished_at_epoch, outcome, error],
        )?;
        Ok(())
    }

    #[test]
    fn never_run_is_ok_without_first_install_noise() -> rusqlite::Result<()> {
        let conn = Connection::open_in_memory()?;
        create_schema(&conn)?;

        let check = check_cleanup_status_at(Some(&conn), NOW);

        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("never_run"), "{}", check.detail);
        Ok(())
    }

    #[test]
    fn active_cleanup_is_informational_and_does_not_read_payload() -> rusqlite::Result<()> {
        let conn = Connection::open_in_memory()?;
        create_schema(&conn)?;
        conn.execute(
            "INSERT INTO jobs
             (job_type, state, payload_json, next_retry_epoch, updated_at_epoch)
             VALUES ('cleanup', 'pending', 'secret-pending-payload', ?1, ?2)",
            rusqlite::params![NOW + 60, NOW],
        )?;
        conn.execute(
            "INSERT INTO jobs
             (job_type, state, payload_json, lease_expires_epoch, updated_at_epoch)
             VALUES ('cleanup', 'processing', 'secret-processing-payload', ?1, ?2)",
            rusqlite::params![NOW + 60, NOW],
        )?;
        conn.execute(
            "INSERT INTO jobs (job_type, state, payload_json, updated_at_epoch)
             VALUES ('other', 'pending', 'other-payload', 1)",
            [],
        )?;

        let check = check_cleanup_status_at(Some(&conn), NOW);

        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("active"), "{}", check.detail);
        assert!(check.detail.contains("scheduled=1"), "{}", check.detail);
        assert!(check.detail.contains("processing=1"), "{}", check.detail);
        assert!(!check.detail.contains("payload"), "{}", check.detail);
        Ok(())
    }

    #[test]
    fn newer_failure_warns_with_bounded_redacted_error() -> rusqlite::Result<()> {
        let conn = Connection::open_in_memory()?;
        create_schema(&conn)?;
        insert_run(&conn, "automatic", "success", NOW - 120, None)?;
        let error = format!("api_key=super-secret-{}", "x".repeat(400));
        insert_run(&conn, "automatic", "failure", NOW - 60, Some(&error))?;

        let check = check_cleanup_status_at(Some(&conn), NOW);

        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("failed"), "{}", check.detail);
        assert!(check.detail.contains("[REDACTED]"), "{}", check.detail);
        assert!(!check.detail.contains("super-secret"), "{}", check.detail);
        assert!(check.detail.len() < 500, "{}", check.detail.len());
        Ok(())
    }

    #[test]
    fn active_retry_is_ok_even_when_failure_is_newest() -> rusqlite::Result<()> {
        let conn = Connection::open_in_memory()?;
        create_schema(&conn)?;
        insert_run(&conn, "automatic", "success", NOW - 120, None)?;
        insert_run(&conn, "automatic", "failure", NOW - 60, Some("transient"))?;
        conn.execute(
            "INSERT INTO jobs
             (job_type, state, payload_json, next_retry_epoch, updated_at_epoch)
             VALUES ('cleanup', 'pending', '{}', ?1, ?2)",
            rusqlite::params![NOW + 30, NOW - 60],
        )?;

        let check = check_cleanup_status_at(Some(&conn), NOW);

        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("active"), "{}", check.detail);
        assert!(check.detail.contains("retrying"), "{}", check.detail);
        Ok(())
    }

    #[test]
    fn stale_pending_and_expired_processing_cleanup_warn() -> rusqlite::Result<()> {
        let conn = Connection::open_in_memory()?;
        create_schema(&conn)?;
        conn.execute(
            "INSERT INTO jobs
             (job_type, state, payload_json, next_retry_epoch, updated_at_epoch)
             VALUES ('cleanup', 'pending', '{}', ?1, ?2)",
            rusqlite::params![NOW - 1, NOW - CLEANUP_STALLED_SECS - 1],
        )?;
        conn.execute(
            "INSERT INTO jobs
             (job_type, state, payload_json, lease_expires_epoch, updated_at_epoch)
             VALUES ('cleanup', 'processing', '{}', ?1, ?2)",
            rusqlite::params![NOW - 1, NOW - CLEANUP_STALLED_SECS],
        )?;

        let check = check_cleanup_status_at(Some(&conn), NOW);

        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("stalled"), "{}", check.detail);
        assert!(
            check.detail.contains("ready_pending_older_than_21600s=1"),
            "{}",
            check.detail
        );
        assert!(
            check.detail.contains("expired_processing=1"),
            "{}",
            check.detail
        );
        Ok(())
    }

    #[test]
    fn newer_success_recovers_and_keeps_old_failure_time() -> rusqlite::Result<()> {
        let conn = Connection::open_in_memory()?;
        create_schema(&conn)?;
        insert_run(&conn, "automatic", "failure", NOW - 120, Some("transient"))?;
        insert_run(&conn, "automatic", "success", NOW - 60, None)?;

        let check = check_cleanup_status_at(Some(&conn), NOW);

        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("healthy"), "{}", check.detail);
        assert!(check.detail.contains("last_failure="), "{}", check.detail);
        assert!(check.detail.contains("superseded"), "{}", check.detail);
        Ok(())
    }

    #[test]
    fn overdue_success_warns_after_cooldown_and_grace() -> rusqlite::Result<()> {
        let conn = Connection::open_in_memory()?;
        create_schema(&conn)?;
        insert_run(
            &conn,
            "automatic",
            "success",
            NOW - CLEANUP_OVERDUE_SECS - 1,
            None,
        )?;

        let check = check_cleanup_status_at(Some(&conn), NOW);

        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("overdue"), "{}", check.detail);
        assert!(
            check.detail.contains(&CLEANUP_OVERDUE_SECS.to_string()),
            "{}",
            check.detail
        );
        Ok(())
    }

    #[test]
    fn missing_schema_or_table_is_ok_and_unavailable() -> rusqlite::Result<()> {
        let empty = Connection::open_in_memory()?;
        let empty_check = check_cleanup_status_at(Some(&empty), NOW);
        assert_eq!(empty_check.status, Status::Ok);
        assert!(
            empty_check.detail.contains("unavailable"),
            "{}",
            empty_check.detail
        );

        let partial = Connection::open_in_memory()?;
        partial.execute_batch(
            "CREATE TABLE maintenance_runs (
                 id INTEGER PRIMARY KEY,
                 \"trigger\" TEXT,
                 started_at_epoch INTEGER,
                 finished_at_epoch INTEGER,
                 outcome TEXT,
                 error TEXT
             );",
        )?;
        let partial_check = check_cleanup_status_at(Some(&partial), NOW);
        assert_eq!(partial_check.status, Status::Ok);
        assert!(partial_check.detail.contains("unavailable"));
        Ok(())
    }

    #[test]
    fn manual_failure_does_not_override_automatic_success() -> rusqlite::Result<()> {
        let conn = Connection::open_in_memory()?;
        create_schema(&conn)?;
        insert_run(&conn, "automatic", "success", NOW - 60, None)?;
        insert_run(&conn, "manual", "failure", NOW - 1, Some("manual-only"))?;

        let check = check_cleanup_status_at(Some(&conn), NOW);

        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("healthy"), "{}", check.detail);
        assert!(!check.detail.contains("manual-only"), "{}", check.detail);
        Ok(())
    }

    #[test]
    fn malformed_schema_warns_without_full_database_error() -> rusqlite::Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE maintenance_runs (id INTEGER PRIMARY KEY);
             CREATE TABLE jobs (id INTEGER PRIMARY KEY);",
        )?;

        let check = check_cleanup_status_at(Some(&conn), NOW);

        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("query failed"), "{}", check.detail);
        assert!(!check.detail.contains("no such column"), "{}", check.detail);
        Ok(())
    }
}
