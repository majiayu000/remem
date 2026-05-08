use rusqlite::{params, Connection};

use super::{claim_next_job, enqueue_job, mark_job_failed_or_retry, JobType};
use crate::migrate::MIGRATIONS;

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    conn.execute_batch(MIGRATIONS[0].sql)
        .expect("baseline schema should load");
    conn
}

#[test]
fn enqueue_job_dedups_inflight_job() {
    let conn = setup_conn();
    let first = enqueue_job(&conn, JobType::Summary, "alpha", Some("s1"), "{}", 100)
        .expect("first enqueue should succeed");
    let second = enqueue_job(&conn, JobType::Summary, "alpha", Some("s1"), "{}", 100)
        .expect("second enqueue should dedup");

    assert_eq!(first, second);
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
        .expect("job count should load");
    assert_eq!(count, 1);
}

#[test]
fn claim_next_job_picks_highest_priority_ready_job() {
    let mut conn = setup_conn();
    let low = enqueue_job(&conn, JobType::Summary, "alpha", Some("s1"), "{}", 200)
        .expect("low priority enqueue should succeed");
    let high = enqueue_job(&conn, JobType::Observation, "alpha", Some("s2"), "{}", 50)
        .expect("high priority enqueue should succeed");
    conn.execute(
        "UPDATE jobs SET next_retry_epoch = ?2 WHERE id = ?1",
        params![low, chrono::Utc::now().timestamp() + 3600],
    )
    .expect("low priority job should be delayed");

    let claimed = claim_next_job(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("one job should be available");

    assert_eq!(claimed.id, high);
    assert_eq!(claimed.job_type, JobType::Observation);
    let state: String = conn
        .query_row(
            "SELECT state FROM jobs WHERE id = ?1",
            params![high],
            |row| row.get(0),
        )
        .expect("claimed job state should load");
    assert_eq!(state, "processing");
}

#[test]
fn mark_job_failed_or_retry_requeues_before_max_attempts() {
    let mut conn = setup_conn();
    let job_id = enqueue_job(&conn, JobType::Summary, "alpha", Some("s1"), "{}", 100)
        .expect("job enqueue should succeed");
    let claimed = claim_next_job(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("job should be claimed");

    mark_job_failed_or_retry(&conn, claimed.id, "worker-a", "boom", 30)
        .expect("retry should succeed");

    let row = conn
        .query_row(
            "SELECT state, attempt_count, lease_owner, next_retry_epoch, last_error
             FROM jobs WHERE id = ?1",
            params![job_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .expect("job row should load");
    assert_eq!(row.0, "pending");
    assert_eq!(row.1, 1);
    assert_eq!(row.2, None);
    assert!(row.3 >= chrono::Utc::now().timestamp() + 29);
    assert_eq!(row.4.as_deref(), Some("boom"));
}

#[test]
fn mark_job_failed_or_retry_marks_failed_when_exhausted() {
    let mut conn = setup_conn();
    let job_id = enqueue_job(&conn, JobType::Summary, "alpha", Some("s1"), "{}", 100)
        .expect("job enqueue should succeed");
    conn.execute(
        "UPDATE jobs SET attempt_count = 5, max_attempts = 6 WHERE id = ?1",
        params![job_id],
    )
    .expect("job attempts should update");
    let claimed = claim_next_job(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("job should be claimed");

    mark_job_failed_or_retry(&conn, claimed.id, "worker-a", "fatal", 30)
        .expect("failure should succeed");

    let row = conn
        .query_row(
            "SELECT state, attempt_count, lease_owner, next_retry_epoch, last_error
             FROM jobs WHERE id = ?1",
            params![job_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .expect("job row should load");
    assert_eq!(row.0, "failed");
    assert_eq!(row.1, 6);
    assert_eq!(row.2, None);
    assert!(row.3 >= 0);
    assert_eq!(row.4.as_deref(), Some("fatal"));
}
