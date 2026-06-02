use rusqlite::params;

use crate::db::test_support::ScopedTestDataDir;
use crate::{db, memory};

use super::database::{check_database, check_pending_queue, check_worker_daemon};
use super::health_action::{queue_actions, render_action_block};
use super::report::{run_doctor_with_writer, DoctorOptions};
use super::schema::check_schema_migration;

#[test]
fn check_database_reports_shared_active_memory_count() {
    let _test_dir = ScopedTestDataDir::new("doctor-db");
    let conn = db::open_db().expect("db should open");
    memory::insert_memory(
        &conn,
        Some("session-1"),
        "proj-a",
        None,
        "active",
        "kept",
        "decision",
        None,
    )
    .expect("active memory insert should succeed");
    let archived_id = memory::insert_memory(
        &conn,
        Some("session-2"),
        "proj-a",
        None,
        "archived",
        "hidden",
        "decision",
        None,
    )
    .expect("archived memory insert should succeed");
    conn.execute(
        "UPDATE memories SET status = 'archived' WHERE id = ?1",
        params![archived_id],
    )
    .expect("archive update should succeed");

    let stats = db::query_system_stats(&conn).expect("system stats should load");
    drop(conn);

    let check = check_database();
    assert_eq!(check.icon(), "ok");
    assert!(check
        .detail
        .contains(&format!("{} memories", stats.active_memories)));
}

#[test]
fn health_action_queue_actions_are_empty_when_runtime_is_clear() {
    let actions = queue_actions(0, 0, 0, 0);
    assert!(actions.is_empty());
    assert!(render_action_block(&actions).is_empty());
}

#[test]
fn health_action_queue_actions_render_copy_paste_commands() {
    let actions = queue_actions(43, 1, 2, 3);
    let text = render_action_block(&actions);

    assert!(text.contains("Needs attention:"));
    assert!(text.contains("43 failed pending observations"));
    assert!(text.contains("inspect: remem pending list-failed --limit 20"));
    assert!(text.contains("preview retry: remem pending retry-failed --dry-run"));
    assert!(text.contains("1 expired processing pending observation"));
    assert!(text.contains("2 failed jobs"));
    assert!(text.contains("3 stuck jobs"));
    assert!(text.contains("inspect counts: remem status --json"));
    assert!(text.contains("recover: remem worker --once"));
}

#[test]
fn check_pending_queue_reports_shared_counts() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-pending");
    let conn = db::open_db().expect("db should open");
    db::enqueue_pending(
        &conn,
        "codex-cli",
        "session-1",
        "proj-a",
        "tool",
        None,
        None,
        None,
    )
    .expect("pending row insert should succeed");
    let failed_id = db::enqueue_pending(
        &conn,
        "codex-cli",
        "session-2",
        "proj-a",
        "tool",
        None,
        None,
        None,
    )
    .expect("failed row insert should succeed");
    conn.execute(
        "UPDATE pending_observations SET status = 'failed' WHERE id = ?1",
        params![failed_id],
    )
    .expect("failed status update should succeed");

    let job_id = db::enqueue_job(
        &conn,
        "codex-cli",
        db::JobType::Observation,
        "proj-a",
        Some("session-3"),
        "{}",
        1,
    )
    .expect("job insert should succeed");
    conn.execute(
        "UPDATE jobs SET state = 'processing', lease_expires_epoch = ?2 WHERE id = ?1",
        params![job_id, chrono::Utc::now().timestamp() - 1],
    )
    .expect("job update should succeed");
    let failed_job_id = db::enqueue_job(
        &conn,
        "codex-cli",
        db::JobType::Summary,
        "proj-a",
        Some("session-4"),
        "{}",
        1,
    )?;
    conn.execute(
        "UPDATE jobs SET state = 'failed' WHERE id = ?1",
        params![failed_job_id],
    )?;

    let stats = db::query_system_stats(&conn).expect("system stats should load");
    drop(conn);

    let check = check_pending_queue();
    assert_eq!(check.icon(), "WARN");
    let expected_counts = format!(
        "{} ready, {} delayed, {} processing ({} expired), {} failed pending; {} jobs pending, {} processing, {} failed, {} stuck",
        stats.ready_pending_observations,
        stats.delayed_pending_observations,
        stats.processing_pending_observations,
        stats.expired_processing_pending_observations,
        stats.failed_pending_observations,
        stats.pending_jobs,
        stats.processing_jobs,
        stats.failed_jobs,
        stats.stuck_jobs,
    );
    assert!(check.detail.contains(&expected_counts), "{}", check.detail);
    assert!(
        check.detail.contains("will auto-recover"),
        "{}",
        check.detail
    );
    assert!(
        check
            .detail
            .contains("inspect: `remem pending list-failed --limit 20`"),
        "{}",
        check.detail
    );
    assert!(
        check
            .detail
            .contains("preview retry: `remem pending retry-failed --dry-run`"),
        "{}",
        check.detail
    );
    assert!(
        check
            .detail
            .contains("inspect counts: `remem status --json`"),
        "{}",
        check.detail
    );
    assert!(
        check.detail.contains("recover: `remem worker --once`"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn check_schema_migration_reads_encrypted_database() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("doctor-encrypted-schema");
    std::fs::create_dir_all(&test_dir.path)?;
    std::fs::write(test_dir.path.join(".key"), "doctor-schema-key")?;
    let conn = db::open_db()?;
    drop(conn);

    let check = check_schema_migration();
    assert_eq!(check.icon(), "ok");
    assert!(check.detail.contains("up to date"), "got: {}", check.detail);
    Ok(())
}

#[test]
fn run_doctor_with_writer_returns_outcome_and_emits_human_lines() {
    let _test_dir = ScopedTestDataDir::new("doctor-run-human");
    // Ensure DB exists so the database probe doesn't FAIL the run.
    let _ = db::open_db().expect("db should open");

    let mut buf: Vec<u8> = Vec::new();
    let outcome = run_doctor_with_writer(DoctorOptions::default(), &mut buf)
        .expect("run_doctor_with_writer should succeed");

    let text = String::from_utf8(buf).expect("output should be utf-8");
    assert!(text.contains("system check"));
    assert!(text.contains("Database"));
    // Exit code is a function of fails/warns; the absolute counts depend on
    // host config (claude/codex hooks may or may not exist on the test
    // machine), but the contract — fails maps to exit 2 — must hold.
    if outcome.fails > 0 {
        assert_eq!(outcome.exit_code(), 2);
    }
}

#[test]
fn run_doctor_with_writer_emits_parseable_json() {
    let _test_dir = ScopedTestDataDir::new("doctor-run-json");
    let _ = db::open_db().expect("db should open");

    let mut buf: Vec<u8> = Vec::new();
    let outcome = run_doctor_with_writer(
        DoctorOptions {
            json: true,
            quiet: false,
        },
        &mut buf,
    )
    .expect("run_doctor_with_writer should succeed in json mode");

    let text = String::from_utf8(buf).expect("output should be utf-8");
    let parsed: serde_json::Value =
        serde_json::from_str(text.trim()).expect("output must be a single JSON object");
    assert!(parsed["version"].is_string());
    assert!(parsed["status"].is_string());
    let checks = parsed["checks"].as_array().expect("checks must be array");
    assert!(!checks.is_empty(), "doctor should always emit some checks");
    assert_eq!(
        parsed["fails"].as_u64().unwrap_or(0) as usize,
        outcome.fails
    );
    assert_eq!(
        parsed["warns"].as_u64().unwrap_or(0) as usize,
        outcome.warns
    );
}

#[test]
fn run_doctor_with_writer_quiet_suppresses_output() {
    let _test_dir = ScopedTestDataDir::new("doctor-run-quiet");
    let _ = db::open_db().expect("db should open");

    let mut buf: Vec<u8> = Vec::new();
    let _outcome = run_doctor_with_writer(
        DoctorOptions {
            json: false,
            quiet: true,
        },
        &mut buf,
    )
    .expect("run_doctor_with_writer should succeed in quiet mode");

    assert!(buf.is_empty(), "quiet mode must not write to stdout");
}

#[test]
fn check_worker_daemon_reports_healthy_heartbeat() {
    let _test_dir = ScopedTestDataDir::new("doctor-worker-healthy");
    let conn = db::open_db().expect("db should open");
    let now = chrono::Utc::now().timestamp();
    db::upsert_worker_heartbeat(
        &conn,
        "worker-daemon",
        i64::from(std::process::id()),
        now - 5,
        now - 5,
    )
    .expect("heartbeat should insert");
    drop(conn);

    let check = check_worker_daemon();
    assert_eq!(check.icon(), "ok");
    assert!(check.detail.contains("healthy"));
    assert!(check.detail.contains("worker-daemon"));
}

#[test]
fn check_worker_daemon_reports_missing_as_fallback_ok() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-worker-missing");
    let conn = db::open_db()?;
    drop(conn);

    let check = check_worker_daemon();
    assert_eq!(check.icon(), "ok");
    assert_eq!(
        check.detail,
        "not running; safe fallback when Stop hooks are installed: `remem worker --once`"
    );
    Ok(())
}
