use crate::db::{
    count_retryable_extraction_replay_ranges, list_extraction_replay_ranges, record_captured_event,
    retry_extraction_replay_ranges, CaptureEventInput,
};
use anyhow::Result;
use rusqlite::{params, Connection};

use super::*;

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    crate::migrate::run_migrations(&conn).expect("migrations should run");
    conn
}

fn insert_task(conn: &Connection, session_id: &str, task_kind: ExtractionTaskKind) -> Result<i64> {
    let outcome = record_captured_event(
        conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id,
            project: "/tmp/remem",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Task"),
            content: session_id,
            task_kind: Some(task_kind),
        },
    )?;
    outcome
        .extraction_task_id
        .ok_or_else(|| anyhow::anyhow!("expected extraction task id"))
}

fn task_status(conn: &Connection, task_id: i64) -> (String, i64, Option<i64>, Option<String>) {
    conn.query_row(
        "SELECT status, attempts, next_retry_epoch, last_error
         FROM extraction_tasks WHERE id = ?1",
        params![task_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .expect("task state should query")
}

#[test]
fn mark_extraction_task_failed_or_retry_fails_permanent_error_without_retry() {
    let mut conn = setup_conn();
    let task_id = insert_task(
        &conn,
        "sess-permanent-retry",
        ExtractionTaskKind::SessionRollup,
    )
    .expect("task should insert");
    claim_next_extraction_task(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("task should be claimed");

    mark_extraction_task_failed_or_retry(&conn, task_id, "worker-a", "not implemented", 30)
        .expect("permanent failure should succeed");

    let (status, attempts, next_retry, last_error) = task_status(&conn, task_id);
    assert_eq!(status, "failed");
    assert_eq!(attempts, 1);
    assert!(next_retry.is_none());
    assert_eq!(last_error.as_deref(), Some("not implemented"));
}

#[test]
fn retry_extraction_replay_ranges_skips_archived_ranges() {
    let mut conn = setup_conn();
    let task_id = insert_task(
        &conn,
        "sess-archived-replay",
        ExtractionTaskKind::ObservationExtract,
    )
    .expect("task should insert");
    conn.execute(
        "UPDATE extraction_tasks SET attempts = ?1 WHERE id = ?2",
        params![EXTRACTION_TASK_MAX_ATTEMPTS - 1, task_id],
    )
    .expect("attempt count should update");
    let claimed = claim_next_extraction_task(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("task should be claimed");
    defer_claimed_extraction_task(&conn, &claimed, "worker-a", "bad model output", 30)
        .expect("defer should exhaust");
    let range = list_extraction_replay_ranges(&conn, None, 10)
        .expect("ranges should list")
        .pop()
        .expect("range should exist");
    conn.execute(
        "UPDATE extraction_replay_ranges
         SET status = 'failed',
             archived_at_epoch = ?1
         WHERE id = ?2",
        params![chrono::Utc::now().timestamp() - 1, range.id],
    )
    .expect("range should archive");

    assert_eq!(
        count_retryable_extraction_replay_ranges(&conn, None, 10).expect("count should succeed"),
        0
    );
    assert_eq!(
        retry_extraction_replay_ranges(&conn, None, 10).expect("retry should skip archived"),
        0
    );
}
