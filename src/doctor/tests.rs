use rusqlite::params;

use crate::db::test_support::ScopedTestDataDir;
use crate::{db, memory};

use super::database::{check_database, check_pending_queue};
use super::report::{run_doctor_with_writer, DoctorOptions};

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
fn check_pending_queue_reports_shared_counts() {
    let _test_dir = ScopedTestDataDir::new("doctor-pending");
    let conn = db::open_db().expect("db should open");
    db::enqueue_pending(&conn, "session-1", "proj-a", "tool", None, None, None)
        .expect("pending row insert should succeed");
    let failed_id = db::enqueue_pending(&conn, "session-2", "proj-a", "tool", None, None, None)
        .expect("failed row insert should succeed");
    conn.execute(
        "UPDATE pending_observations SET status = 'failed' WHERE id = ?1",
        params![failed_id],
    )
    .expect("failed status update should succeed");

    let job_id = db::enqueue_job(
        &conn,
        db::JobType::Observation,
        "proj-a",
        Some("session-3"),
        "{}",
        1,
    )
    .expect("job insert should succeed");
    conn.execute(
        "UPDATE jobs SET state = 'running', lease_expires_epoch = ?2 WHERE id = ?1",
        params![job_id, chrono::Utc::now().timestamp() - 1],
    )
    .expect("job update should succeed");

    let stats = db::query_system_stats(&conn).expect("system stats should load");
    drop(conn);

    let check = check_pending_queue();
    assert_eq!(check.icon(), "WARN");
    assert_eq!(
        check.detail,
        format!(
            "{} pending, {} failed, {} stuck jobs (will auto-recover)",
            stats.pending_observations, stats.failed_pending_observations, stats.stuck_jobs
        )
    );
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
    assert_eq!(parsed["fails"].as_u64().unwrap_or(0) as usize, outcome.fails);
    assert_eq!(parsed["warns"].as_u64().unwrap_or(0) as usize, outcome.warns);
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
