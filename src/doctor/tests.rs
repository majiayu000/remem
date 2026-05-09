use rusqlite::params;

use crate::db::test_support::ScopedTestDataDir;
use crate::{db, memory};

use super::database::{check_database, check_pending_queue, check_worker_daemon};

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

    let stats = db::query_system_stats(&conn).expect("system stats should load");
    drop(conn);

    let check = check_pending_queue();
    assert_eq!(check.icon(), "WARN");
    assert_eq!(
        check.detail,
        format!(
            "{} ready, {} delayed, {} processing ({} expired), {} failed pending; {} jobs pending, {} processing, {} failed, {} stuck (will auto-recover)",
            stats.ready_pending_observations,
            stats.delayed_pending_observations,
            stats.processing_pending_observations,
            stats.expired_processing_pending_observations,
            stats.failed_pending_observations,
            stats.pending_jobs,
            stats.processing_jobs,
            stats.failed_jobs,
            stats.stuck_jobs,
        )
    );
}

#[test]
fn check_worker_daemon_reports_healthy_heartbeat() {
    let _test_dir = ScopedTestDataDir::new("doctor-worker-healthy");
    let conn = db::open_db().expect("db should open");
    let now = chrono::Utc::now().timestamp();
    db::upsert_worker_heartbeat(&conn, "worker-daemon", 123, now - 5, now - 5)
        .expect("heartbeat should insert");
    drop(conn);

    let check = check_worker_daemon();
    assert_eq!(check.icon(), "ok");
    assert!(check.detail.contains("healthy"));
    assert!(check.detail.contains("worker-daemon"));
}

#[test]
fn check_worker_daemon_reports_missing_as_fallback_ok() {
    let _test_dir = ScopedTestDataDir::new("doctor-worker-missing");

    let check = check_worker_daemon();
    assert_eq!(check.icon(), "ok");
    assert_eq!(
        check.detail,
        "not running; Stop hooks will use worker --once"
    );
}
