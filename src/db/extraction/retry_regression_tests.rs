use crate::db::{
    count_retryable_extraction_replay_ranges, get_extraction_replay_range_evidence,
    list_extraction_replay_ranges, mark_replay_range_replayed_if_done,
    quarantine_extraction_replay_range, record_captured_event, retry_extraction_replay_range,
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

fn exhaust_task_into_replay_range(conn: &mut Connection, session_id: &str) -> i64 {
    let task_id = insert_task(conn, session_id, ExtractionTaskKind::ObservationExtract)
        .expect("task should insert");
    conn.execute(
        "UPDATE extraction_tasks SET attempts = ?1 WHERE id = ?2",
        params![EXTRACTION_TASK_MAX_ATTEMPTS - 1, task_id],
    )
    .expect("attempt count should update");
    let claimed = claim_next_extraction_task(conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("task should be claimed");
    defer_claimed_extraction_task(conn, &claimed, "worker-a", "bad model output", 30)
        .expect("defer should exhaust");
    list_extraction_replay_ranges(conn, None, 10)
        .expect("ranges should list")
        .into_iter()
        .find(|range| range.source_task_id == task_id)
        .expect("range should exist")
        .id
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

#[test]
fn exact_replay_range_operations_do_not_mutate_sibling_ranges() {
    let mut conn = setup_conn();
    let retry_id = exhaust_task_into_replay_range(&mut conn, "sess-exact-retry");
    let quarantine_id = exhaust_task_into_replay_range(&mut conn, "sess-exact-quarantine");

    retry_extraction_replay_range(&conn, retry_id).expect("exact retry should enqueue");
    let ranges = list_extraction_replay_ranges(&conn, None, 10).expect("ranges should list");
    assert_eq!(
        ranges
            .iter()
            .find(|range| range.id == retry_id)
            .map(|range| range.status.as_str()),
        Some("requeued")
    );
    assert_eq!(
        ranges
            .iter()
            .find(|range| range.id == quarantine_id)
            .map(|range| range.status.as_str()),
        Some("pending")
    );

    quarantine_extraction_replay_range(&conn, quarantine_id)
        .expect("exact quarantine should succeed");
    let ranges = list_extraction_replay_ranges(&conn, None, 10).expect("ranges should list");
    assert_eq!(
        ranges
            .iter()
            .find(|range| range.id == retry_id)
            .map(|range| range.status.as_str()),
        Some("requeued")
    );
    assert_eq!(
        ranges
            .iter()
            .find(|range| range.id == quarantine_id)
            .map(|range| range.status.as_str()),
        Some("quarantined")
    );
}

#[test]
fn exact_range_list_includes_replayed_task_evidence() {
    let mut conn = setup_conn();
    let range_id = exhaust_task_into_replay_range(&mut conn, "sess-exact-list");
    retry_extraction_replay_range(&conn, range_id).expect("exact retry should enqueue");
    let pending = get_extraction_replay_range_evidence(&conn, range_id)
        .expect("exact pending evidence should query");
    let replay_task_id = pending
        .replay_task
        .expect("replay task evidence should exist")
        .id;
    conn.execute(
        "UPDATE extraction_tasks SET status = 'done' WHERE id = ?1",
        params![replay_task_id],
    )
    .expect("replay task should complete");
    mark_replay_range_replayed_if_done(&conn, replay_task_id, chrono::Utc::now().timestamp())
        .expect("range should become replayed");

    let evidence = get_extraction_replay_range_evidence(&conn, range_id)
        .expect("terminal exact evidence should remain queryable");
    assert_eq!(evidence.range.status, "replayed");
    let task = evidence
        .replay_task
        .expect("terminal replay task should remain");
    assert_eq!(task.id, replay_task_id);
    assert_eq!(task.status, "done");
    assert_eq!(task.attempts, 0);
    assert!(task.last_error.is_none());
}

#[test]
fn exact_range_operations_reject_non_positive_ids() {
    let conn = setup_conn();
    for range_id in [0, -1] {
        assert!(get_extraction_replay_range_evidence(&conn, range_id).is_err());
        assert!(retry_extraction_replay_range(&conn, range_id).is_err());
        assert!(quarantine_extraction_replay_range(&conn, range_id).is_err());
    }
}

#[test]
fn exact_range_operations_reject_missing_archived_active_and_terminal_targets() {
    let mut conn = setup_conn();
    assert!(get_extraction_replay_range_evidence(&conn, i64::MAX).is_err());
    assert!(retry_extraction_replay_range(&conn, i64::MAX).is_err());

    let archived_id = exhaust_task_into_replay_range(&mut conn, "sess-exact-archived");
    let active_id = exhaust_task_into_replay_range(&mut conn, "sess-exact-active");
    let terminal_id = exhaust_task_into_replay_range(&mut conn, "sess-exact-terminal");
    conn.execute(
        "UPDATE extraction_replay_ranges SET archived_at_epoch = 1 WHERE id = ?1",
        params![archived_id],
    )
    .expect("archive exact target");
    assert!(retry_extraction_replay_range(&conn, archived_id).is_err());
    assert!(quarantine_extraction_replay_range(&conn, archived_id).is_err());

    retry_extraction_replay_range(&conn, active_id).expect("enqueue exact active target");
    assert!(retry_extraction_replay_range(&conn, active_id).is_err());
    assert!(quarantine_extraction_replay_range(&conn, active_id).is_err());

    quarantine_extraction_replay_range(&conn, terminal_id).expect("quarantine exact target");
    assert!(retry_extraction_replay_range(&conn, terminal_id).is_err());
    assert_eq!(
        get_extraction_replay_range_evidence(&conn, terminal_id)
            .expect("terminal evidence remains queryable")
            .range
            .status,
        "quarantined"
    );
}
