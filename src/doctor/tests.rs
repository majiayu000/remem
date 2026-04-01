use rusqlite::params;

use crate::db::test_support::ScopedTestDataDir;
use crate::{db, memory};

use super::database::{check_database, check_pending_queue};

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
