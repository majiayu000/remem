use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::db;
use crate::db::test_support::ScopedTestDataDir;

use super::{run_migrations, MIGRATIONS};

const SUMMARY_RETIREMENT_MARKER: &str = "legacy summary job rejected during GH684 summary retirement upgrade; SessionRollup owns session summary output";
const DUPLICATE_MARKER: &str = "[job_queue_atomicity_migration_duplicate ";

#[derive(Clone)]
struct JobFixture {
    id: i64,
    host: String,
    job_type: String,
    project: String,
    session_id: Option<String>,
    payload_json: String,
    state: String,
    priority: i64,
    attempt_count: i64,
    max_attempts: i64,
    lease_owner: Option<String>,
    lease_expires_epoch: Option<i64>,
    next_retry_epoch: i64,
    last_error: Option<String>,
    created_at_epoch: i64,
    updated_at_epoch: i64,
    failure_class: Option<String>,
    failed_at_epoch: Option<i64>,
    archived_at_epoch: Option<i64>,
}

impl JobFixture {
    fn new(id: i64, job_type: &str, project: &str, state: &str) -> Self {
        Self {
            id,
            host: "codex-cli".into(),
            job_type: job_type.into(),
            project: project.into(),
            session_id: Some("session-a".into()),
            payload_json: "{}".into(),
            state: state.into(),
            priority: 100,
            attempt_count: 0,
            max_attempts: 6,
            lease_owner: None,
            lease_expires_epoch: None,
            next_retry_epoch: 0,
            last_error: None,
            created_at_epoch: 1_700_000_000 + id,
            updated_at_epoch: 1_700_000_000 + id,
            failure_class: None,
            failed_at_epoch: None,
            archived_at_epoch: None,
        }
    }
}

#[derive(Debug, PartialEq)]
struct JobSnapshot {
    host: String,
    payload_json: String,
    state: String,
    priority: i64,
    attempt_count: i64,
    max_attempts: i64,
    lease_owner: Option<String>,
    lease_expires_epoch: Option<i64>,
    next_retry_epoch: i64,
    last_error: Option<String>,
    created_at_epoch: i64,
    updated_at_epoch: i64,
    failure_class: Option<String>,
    failed_at_epoch: Option<i64>,
    archived_at_epoch: Option<i64>,
}

fn pre_v069(label: &str) -> Result<(ScopedTestDataDir, Connection)> {
    let data_dir = ScopedTestDataDir::new(label);
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;
    conn.execute("DELETE FROM _schema_migrations WHERE version = 69", [])?;
    conn.execute_batch(
        "DROP INDEX idx_jobs_active_ordinary_unique;
         DROP INDEX idx_jobs_active_dream_unique;
         DROP INDEX idx_jobs_active_compile_rules_unique;",
    )?;
    Ok((data_dir, conn))
}

fn insert_job(conn: &Connection, job: &JobFixture) -> Result<()> {
    conn.execute(
        "INSERT INTO jobs
         (id, host, job_type, project, session_id, payload_json, state, priority,
          attempt_count, max_attempts, lease_owner, lease_expires_epoch,
          next_retry_epoch, last_error, created_at_epoch, updated_at_epoch,
          failure_class, failed_at_epoch, archived_at_epoch)
         VALUES
         (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
          ?15, ?16, ?17, ?18, ?19)",
        params![
            job.id,
            job.host,
            job.job_type,
            job.project,
            job.session_id,
            job.payload_json,
            job.state,
            job.priority,
            job.attempt_count,
            job.max_attempts,
            job.lease_owner,
            job.lease_expires_epoch,
            job.next_retry_epoch,
            job.last_error,
            job.created_at_epoch,
            job.updated_at_epoch,
            job.failure_class,
            job.failed_at_epoch,
            job.archived_at_epoch,
        ],
    )?;
    Ok(())
}

fn snapshot(conn: &Connection, id: i64) -> Result<JobSnapshot> {
    conn.query_row(
        "SELECT host, payload_json, state, priority, attempt_count, max_attempts,
                lease_owner, lease_expires_epoch, next_retry_epoch, last_error,
                created_at_epoch, updated_at_epoch, failure_class,
                failed_at_epoch, archived_at_epoch
         FROM jobs WHERE id = ?1",
        [id],
        |row| {
            Ok(JobSnapshot {
                host: row.get(0)?,
                payload_json: row.get(1)?,
                state: row.get(2)?,
                priority: row.get(3)?,
                attempt_count: row.get(4)?,
                max_attempts: row.get(5)?,
                lease_owner: row.get(6)?,
                lease_expires_epoch: row.get(7)?,
                next_retry_epoch: row.get(8)?,
                last_error: row.get(9)?,
                created_at_epoch: row.get(10)?,
                updated_at_epoch: row.get(11)?,
                failure_class: row.get(12)?,
                failed_at_epoch: row.get(13)?,
                archived_at_epoch: row.get(14)?,
            })
        },
    )
    .map_err(Into::into)
}

fn active_ids(conn: &Connection, job_type: &str, project: &str) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM jobs
         WHERE job_type = ?1 AND project = ?2
           AND state IN ('pending', 'processing')
         ORDER BY id",
    )?;
    let ids = stmt
        .query_map(params![job_type, project], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(anyhow::Error::from)?;
    Ok(ids)
}

fn assert_duplicate_failure(
    conn: &Connection,
    duplicate_id: i64,
    canonical_id: i64,
    identity_kind: &str,
) -> Result<JobSnapshot> {
    let row = snapshot(conn, duplicate_id)?;
    assert_eq!(row.state, "failed");
    assert_eq!(row.failure_class.as_deref(), Some("permanent"));
    assert_eq!(row.next_retry_epoch, 0);
    assert_eq!(row.archived_at_epoch, None);
    let error = row.last_error.as_deref().unwrap_or_default();
    assert!(error.contains(DUPLICATE_MARKER), "got: {error}");
    assert!(
        error.contains(&format!("duplicate_id={duplicate_id}")),
        "got: {error}"
    );
    assert!(
        error.contains(&format!("canonical_id={canonical_id}")),
        "got: {error}"
    );
    assert!(
        error.contains(&format!("identity_kind={identity_kind}")),
        "got: {error}"
    );
    Ok(row)
}

#[test]
fn v069_reconciles_active_job_duplicates_before_unique_indexes() -> Result<()> {
    let (_data_dir, conn) = pre_v069("v069-reconcile")?;

    let mut ordinary_pending = JobFixture::new(101, "compress", "/ordinary", "pending");
    ordinary_pending.session_id = None;
    ordinary_pending.priority = 1;
    let mut ordinary_processing = JobFixture::new(102, "compress", "/ordinary", "processing");
    ordinary_processing.session_id = Some(String::new());
    ordinary_processing.lease_owner = Some("worker-old".into());
    ordinary_processing.lease_expires_epoch = Some(1);
    insert_job(&conn, &ordinary_pending)?;
    insert_job(&conn, &ordinary_processing)?;

    let mut compile_later = JobFixture::new(103, "compile_rules", "/compile", "pending");
    compile_later.priority = 90;
    let mut compile_first = JobFixture::new(104, "compile_rules", "/compile", "pending");
    compile_first.priority = 10;
    insert_job(&conn, &compile_later)?;
    insert_job(&conn, &compile_first)?;

    run_migrations(&conn)?;

    assert_eq!(active_ids(&conn, "compress", "/ordinary")?, vec![102]);
    assert_duplicate_failure(&conn, 101, 102, "ordinary")?;
    assert_eq!(active_ids(&conn, "compile_rules", "/compile")?, vec![104]);
    assert_duplicate_failure(&conn, 103, 104, "compile_rules")?;

    for index in [
        "idx_jobs_active_ordinary_unique",
        "idx_jobs_active_dream_unique",
        "idx_jobs_active_compile_rules_unique",
    ] {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1)",
            [index],
            |row| row.get(0),
        )?;
        assert!(exists, "missing {index}");
    }
    let mut conflicting = JobFixture::new(105, "compress", "/ordinary", "pending");
    conflicting.session_id = None;
    assert!(insert_job(&conn, &conflicting).is_err());
    Ok(())
}

#[test]
fn v069_replays_pending_dream_duplicates_with_current_profile_predicate() -> Result<()> {
    let (_data_dir, conn) = pre_v069("v069-dream-replay")?;
    let cases = [
        (
            "/profile-empty",
            r#"{"remem_ai_profile":"quality","value":"base"}"#,
            r#"{"remem_ai_profile":"   ","value":"empty"}"#,
            40,
            5,
            "host-base",
            "host-empty",
            r#"{"remem_ai_profile":"quality","value":"base"}"#,
            40,
            "host-base",
        ),
        (
            "/empty-profile",
            r#"{"value":"base"}"#,
            r#"{"remem_ai_profile":"quality","value":"incoming"}"#,
            40,
            30,
            "host-base",
            "host-profile",
            r#"{"remem_ai_profile":"quality","value":"incoming"}"#,
            30,
            "host-profile",
        ),
        (
            "/same-profile",
            r#"{"remem_ai_profile":"quality","value":"base"}"#,
            r#"{"remem_ai_profile":" quality ","value":"same"}"#,
            40,
            5,
            "host-base",
            "host-same",
            r#"{"remem_ai_profile":"quality","value":"base"}"#,
            40,
            "host-base",
        ),
        (
            "/different-profile",
            r#"{"remem_ai_profile":"quality","value":"base"}"#,
            r#"{"remem_ai_profile":"fast","value":"incoming"}"#,
            40,
            5,
            "host-base",
            "host-fast",
            r#"{"remem_ai_profile":"fast","value":"incoming"}"#,
            5,
            "host-fast",
        ),
    ];
    for (offset, case) in cases.iter().enumerate() {
        let base_id = 200 + (offset as i64 * 2);
        let mut base = JobFixture::new(base_id, "dream", case.0, "pending");
        base.payload_json = case.1.into();
        base.priority = case.3;
        base.host = case.5.into();
        base.created_at_epoch = base_id;
        let mut incoming = JobFixture::new(base_id + 1, "dream", case.0, "pending");
        incoming.payload_json = case.2.into();
        incoming.priority = case.4;
        incoming.host = case.6.into();
        incoming.created_at_epoch = base_id + 1;
        insert_job(&conn, &base)?;
        insert_job(&conn, &incoming)?;
    }

    run_migrations(&conn)?;

    for (offset, case) in cases.iter().enumerate() {
        let survivor_id = 200 + (offset as i64 * 2);
        assert_eq!(active_ids(&conn, "dream", case.0)?, vec![survivor_id]);
        let survivor = snapshot(&conn, survivor_id)?;
        assert_eq!(survivor.payload_json, case.7);
        assert_eq!(survivor.priority, case.8);
        assert_eq!(survivor.host, case.9);
        assert_duplicate_failure(&conn, survivor_id + 1, survivor_id, "dream")?;
    }
    Ok(())
}

#[test]
fn v069_treats_malformed_non_string_missing_and_blank_dream_profiles_as_empty() -> Result<()> {
    let (_data_dir, conn) = pre_v069("v069-dream-invalid-profiles")?;
    let empty_payloads = [
        "{malformed",
        r#"{"remem_ai_profile":42,"kind":"non-string"}"#,
        r#"{"kind":"missing"}"#,
        r#"{"remem_ai_profile":"  ","kind":"blank"}"#,
    ];
    for (offset, empty_payload) in empty_payloads.iter().enumerate() {
        let base_id = 300 + (offset as i64 * 3);
        let project = format!("/invalid-profile-{offset}");
        let mut empty = JobFixture::new(base_id, "dream", &project, "pending");
        empty.payload_json = (*empty_payload).into();
        empty.host = "host-empty".into();
        empty.priority = 50;
        let mut valid = JobFixture::new(base_id + 1, "dream", &project, "pending");
        valid.payload_json =
            format!(r#"{{"remem_ai_profile":"quality-{offset}","secret":"safe"}}"#);
        valid.host = "host-valid".into();
        valid.priority = 20;
        let mut later_empty = JobFixture::new(base_id + 2, "dream", &project, "pending");
        later_empty.payload_json = (*empty_payload).into();
        later_empty.host = "host-later-empty".into();
        later_empty.priority = 1;
        insert_job(&conn, &empty)?;
        insert_job(&conn, &valid)?;
        insert_job(&conn, &later_empty)?;
    }

    run_migrations(&conn)?;

    for offset in 0..empty_payloads.len() {
        let id = 300 + (offset as i64 * 3);
        let row = snapshot(&conn, id)?;
        assert_eq!(row.host, "host-valid");
        assert_eq!(row.priority, 20);
        assert_eq!(
            row.payload_json,
            format!(r#"{{"remem_ai_profile":"quality-{offset}","secret":"safe"}}"#)
        );
    }
    Ok(())
}

#[test]
fn v069_late_active_summary_uses_v064_marker_and_stays_non_actionable() -> Result<()> {
    let (_data_dir, conn) = pre_v069("v069-late-summary")?;
    let mut active = JobFixture::new(400, "summary", "/summary", "processing");
    active.attempt_count = 2;
    active.lease_owner = Some("old-worker".into());
    active.lease_expires_epoch = Some(i64::MAX);
    let mut terminal = JobFixture::new(401, "summary", "/summary", "failed");
    terminal.attempt_count = 1;
    terminal.last_error = Some("terminal history".into());
    terminal.failure_class = Some("transient".into());
    terminal.failed_at_epoch = Some(11);
    terminal.archived_at_epoch = Some(12);
    insert_job(&conn, &active)?;
    insert_job(&conn, &terminal)?;
    let terminal_before = snapshot(&conn, 401)?;

    run_migrations(&conn)?;

    let retired = snapshot(&conn, 400)?;
    assert_eq!(retired.state, "failed");
    assert_eq!(retired.attempt_count, 6);
    assert_eq!(
        retired.last_error.as_deref(),
        Some(SUMMARY_RETIREMENT_MARKER)
    );
    assert_eq!(retired.failure_class.as_deref(), Some("permanent"));
    assert!(!retired
        .last_error
        .unwrap_or_default()
        .contains(DUPLICATE_MARKER));
    assert_eq!(snapshot(&conn, 401)?, terminal_before);
    let stats = db::query_system_stats(&conn)?;
    assert_eq!(stats.failure_lifecycle.job.actionable_total, 0);
    let summary_surface = stats
        .legacy_surfaces
        .iter()
        .find(|surface| surface.surface == "summary_jobs")
        .context("summary_jobs legacy surface")?;
    assert_eq!(summary_surface.frozen_write_violations, 0);
    Ok(())
}

#[test]
fn v069_does_not_rewrite_processing_dream_payload() -> Result<()> {
    let (_data_dir, conn) = pre_v069("v069-processing-dream")?;
    let mut processing = JobFixture::new(500, "dream", "/dream-processing", "processing");
    processing.host = "processing-host".into();
    processing.payload_json = r#"{"remem_ai_profile":"quality","secret":"keep"}"#.into();
    processing.priority = 60;
    processing.lease_owner = Some("worker-a".into());
    processing.lease_expires_epoch = Some(i64::MAX);
    let mut pending = JobFixture::new(501, "dream", "/dream-processing", "pending");
    pending.host = "pending-host".into();
    pending.payload_json = r#"{"remem_ai_profile":"fast","secret":"do-not-merge"}"#.into();
    pending.priority = 1;
    insert_job(&conn, &processing)?;
    insert_job(&conn, &pending)?;

    run_migrations(&conn)?;

    let survivor = snapshot(&conn, 500)?;
    assert_eq!(survivor.host, "processing-host");
    assert_eq!(
        survivor.payload_json,
        r#"{"remem_ai_profile":"quality","secret":"keep"}"#
    );
    assert_eq!(survivor.priority, 60);
    let duplicate = assert_duplicate_failure(&conn, 501, 500, "dream")?;
    assert!(duplicate
        .last_error
        .unwrap_or_default()
        .contains("manual_review=true"));
    Ok(())
}

#[test]
fn v069_preserves_existing_duplicate_last_error_and_appends_marker() -> Result<()> {
    let (_data_dir, conn) = pre_v069("v069-existing-error")?;
    let mut canonical = JobFixture::new(600, "compress", "/error", "pending");
    canonical.priority = 1;
    let mut duplicate = JobFixture::new(601, "compress", "/error", "pending");
    duplicate.priority = 2;
    duplicate.last_error = Some("original root cause".into());
    insert_job(&conn, &canonical)?;
    insert_job(&conn, &duplicate)?;

    run_migrations(&conn)?;

    let row = assert_duplicate_failure(&conn, 601, 600, "ordinary")?;
    let error = row.last_error.unwrap_or_default();
    assert!(error.starts_with("original root cause "));
    assert!(error.ends_with(']'));
    Ok(())
}

#[test]
fn v069_truncates_near_limit_duplicate_last_error_without_losing_marker() -> Result<()> {
    let (_data_dir, conn) = pre_v069("v069-bounded-error")?;
    let mut canonical = JobFixture::new(610, "compress", "/bounded", "pending");
    canonical.priority = 1;
    let mut duplicate = JobFixture::new(611, "compress", "/bounded", "pending");
    duplicate.priority = 2;
    duplicate.last_error = Some(format!("root-prefix:{}", "x".repeat(2_100)));
    insert_job(&conn, &canonical)?;
    insert_job(&conn, &duplicate)?;

    run_migrations(&conn)?;

    let error = assert_duplicate_failure(&conn, 611, 610, "ordinary")?
        .last_error
        .unwrap_or_default();
    assert!(error.starts_with("root-prefix:"));
    assert!(error.len() <= 2_000, "len={}", error.len());
    assert!(error.ends_with(']'));
    Ok(())
}

#[test]
fn v069_preserves_redundant_active_attempt_count_without_reporting_exhaustion() -> Result<()> {
    let (_data_dir, conn) = pre_v069("v069-attempt-evidence")?;
    let mut canonical = JobFixture::new(620, "compress", "/attempt", "pending");
    canonical.priority = 1;
    let mut duplicate = JobFixture::new(621, "compress", "/attempt", "pending");
    duplicate.priority = 2;
    duplicate.attempt_count = 2;
    duplicate.max_attempts = 6;
    insert_job(&conn, &canonical)?;
    insert_job(&conn, &duplicate)?;

    run_migrations(&conn)?;

    let row = assert_duplicate_failure(&conn, 621, 620, "ordinary")?;
    assert_eq!(row.attempt_count, 2);
    assert_eq!(row.max_attempts, 6);
    let stats = db::query_failure_lifecycle_stats(&conn, chrono::Utc::now().timestamp())?;
    assert_eq!(stats.job.actionable_total, 1);
    assert_eq!(stats.job.permanent, 1);
    assert_eq!(stats.job.exhausted, 0);
    Ok(())
}

#[test]
fn v069_duplicate_failure_enters_actionable_failure_lifecycle() -> Result<()> {
    let (_data_dir, conn) = pre_v069("v069-actionable")?;
    let mut canonical = JobFixture::new(630, "compress", "/actionable", "pending");
    canonical.priority = 1;
    let mut duplicate = JobFixture::new(631, "compress", "/actionable", "pending");
    duplicate.priority = 2;
    insert_job(&conn, &canonical)?;
    insert_job(&conn, &duplicate)?;

    run_migrations(&conn)?;

    let row = assert_duplicate_failure(&conn, 631, 630, "ordinary")?;
    assert!(row.failed_at_epoch.is_some());
    let stats = db::query_system_stats(&conn)?;
    assert_eq!(stats.failed_jobs, 1);
    assert_eq!(stats.failure_lifecycle.job.actionable_total, 1);
    assert_eq!(stats.failure_lifecycle.job.archived, 0);
    Ok(())
}

#[test]
fn v069_normalizes_null_processing_lease_to_expired_before_survivor_selection() -> Result<()> {
    let (_data_dir, conn) = pre_v069("v069-null-lease")?;
    let mut processing = JobFixture::new(640, "compress", "/null-lease", "processing");
    processing.lease_owner = Some("worker-null".into());
    processing.lease_expires_epoch = None;
    processing.attempt_count = 2;
    processing.payload_json = r#"{"secret":"unchanged"}"#.into();
    let mut pending = JobFixture::new(641, "compress", "/null-lease", "pending");
    pending.priority = 1;
    insert_job(&conn, &processing)?;
    insert_job(&conn, &pending)?;

    run_migrations(&conn)?;

    let row = snapshot(&conn, 640)?;
    assert_eq!(row.state, "processing");
    assert_eq!(row.lease_owner.as_deref(), Some("worker-null"));
    assert_eq!(row.attempt_count, 2);
    assert_eq!(row.payload_json, r#"{"secret":"unchanged"}"#);
    assert!(row
        .lease_expires_epoch
        .is_some_and(|expiry| expiry < chrono::Utc::now().timestamp()));
    assert_eq!(db::query_system_stats(&conn)?.stuck_jobs, 1);
    assert_eq!(db::requeue_stuck_jobs(&conn)?, 1);
    assert_eq!(snapshot(&conn, 640)?.state, "pending");
    Ok(())
}

#[test]
fn v069_preserves_terminal_job_history_and_is_idempotent() -> Result<()> {
    let (_data_dir, conn) = pre_v069("v069-terminal-history")?;
    let mut done = JobFixture::new(650, "compress", "/history", "done");
    done.attempt_count = 4;
    done.last_error = Some("done audit".into());
    done.failure_class = Some("permanent".into());
    done.failed_at_epoch = Some(77);
    done.archived_at_epoch = Some(88);
    let mut failed = JobFixture::new(651, "dream", "/history", "failed");
    failed.attempt_count = 2;
    failed.last_error = Some("failed audit".into());
    failed.failure_class = Some("transient".into());
    failed.failed_at_epoch = Some(99);
    failed.archived_at_epoch = Some(111);
    insert_job(&conn, &done)?;
    insert_job(&conn, &failed)?;
    let before = (snapshot(&conn, 650)?, snapshot(&conn, 651)?);

    run_migrations(&conn)?;
    let after_first = (snapshot(&conn, 650)?, snapshot(&conn, 651)?);
    run_migrations(&conn)?;
    let after_second = (snapshot(&conn, 650)?, snapshot(&conn, 651)?);

    assert_eq!(after_first, before);
    assert_eq!(after_second, before);
    let applied: i64 = conn.query_row(
        "SELECT COUNT(*) FROM _schema_migrations WHERE version=69 AND name='job_queue_atomicity'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(applied, 1);
    Ok(())
}

#[test]
fn v069_post_migration_hook_logs_conflict_counts_without_payload() -> Result<()> {
    let (data_dir, conn) = pre_v069("v069-safe-log")?;
    let mut canonical = JobFixture::new(660, "compress", "/log", "pending");
    canonical.priority = 1;
    canonical.payload_json = r#"{"secret":"fixture-secret-token"}"#.into();
    let mut duplicate = JobFixture::new(661, "compress", "/log", "pending");
    duplicate.priority = 2;
    duplicate.last_error = Some("fixture-private-error".into());
    insert_job(&conn, &canonical)?;
    insert_job(&conn, &duplicate)?;

    run_migrations(&conn)?;

    let log = std::fs::read_to_string(data_dir.path.join("remem.log"))?;
    assert!(log.contains("v069 job queue reconciliation ordinary=1 manual_review=0"));
    assert!(log.contains("dream=0 manual_review=0"));
    assert!(log.contains("compile_rules=0 manual_review=0"));
    assert!(!log.contains("fixture-secret-token"));
    assert!(!log.contains("fixture-private-error"));
    assert!(!log.contains("payload_json"));
    Ok(())
}

#[test]
fn v069_post_migration_hook_log_failure_rolls_back_migration() -> Result<()> {
    let (data_dir, conn) = pre_v069("v069-log-rollback")?;
    let mut canonical = JobFixture::new(665, "compress", "/log-rollback", "pending");
    canonical.priority = 1;
    let mut duplicate = JobFixture::new(666, "compress", "/log-rollback", "pending");
    duplicate.priority = 2;
    insert_job(&conn, &canonical)?;
    insert_job(&conn, &duplicate)?;
    let before = (snapshot(&conn, 665)?, snapshot(&conn, 666)?);

    let blocked_log_parent = data_dir.path.join("not-a-directory");
    std::fs::write(&blocked_log_parent, b"block log directory creation")?;
    let error = crate::log::with_log_dir(&blocked_log_parent, || run_migrations(&conn))
        .expect_err("log preparation failure must abort v069");

    assert!(
        format!("{error:#}").contains("failed to log reconciliation counts"),
        "got: {error:#}"
    );
    assert_eq!((snapshot(&conn, 665)?, snapshot(&conn, 666)?), before);
    let applied: i64 = conn.query_row(
        "SELECT COUNT(*) FROM _schema_migrations WHERE version=69",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(applied, 0);
    let index_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type='index' AND name LIKE 'idx_jobs_active_%_unique'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(index_count, 0);
    Ok(())
}

#[test]
fn job_queue_atomicity_migration_rolls_back_all_changes_on_validation_error() -> Result<()> {
    let (_data_dir, conn) = pre_v069("v069-validation-rollback")?;
    let mut canonical = JobFixture::new(670, "compress", "/rollback", "pending");
    canonical.priority = 1;
    let mut duplicate = JobFixture::new(671, "compress", "/rollback", "pending");
    duplicate.priority = 2;
    insert_job(&conn, &canonical)?;
    insert_job(&conn, &duplicate)?;
    let before = (snapshot(&conn, 670)?, snapshot(&conn, 671)?);
    conn.execute_batch(
        "CREATE TRIGGER prevent_v069_duplicate_terminalization
         BEFORE UPDATE OF state ON jobs
         WHEN OLD.state IN ('pending', 'processing')
          AND NEW.state = 'failed'
          AND OLD.job_type = 'compress'
         BEGIN
           SELECT RAISE(IGNORE);
         END;",
    )?;

    let error = run_migrations(&conn).expect_err("validation must reject remaining duplicates");
    assert!(
        format!("{error:#}").contains("CHECK constraint failed"),
        "got: {error:#}"
    );
    assert_eq!((snapshot(&conn, 670)?, snapshot(&conn, 671)?), before);
    let applied: i64 = conn.query_row(
        "SELECT COUNT(*) FROM _schema_migrations WHERE version=69",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(applied, 0);
    for index in [
        "idx_jobs_active_ordinary_unique",
        "idx_jobs_active_dream_unique",
        "idx_jobs_active_compile_rules_unique",
    ] {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1)",
            [index],
            |row| row.get(0),
        )?;
        assert!(!exists, "{index} must roll back");
    }
    Ok(())
}

#[test]
fn v069_migration_is_latest_and_named_stably() {
    let migration = MIGRATIONS.last().expect("v069 migration");
    assert_eq!(migration.version, 69);
    assert_eq!(migration.name, "job_queue_atomicity");
}
