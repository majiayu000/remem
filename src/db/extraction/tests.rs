use crate::db::{
    count_retryable_extraction_replay_ranges, list_extraction_replay_ranges,
    quarantine_extraction_replay_ranges, record_captured_event, retry_extraction_replay_ranges,
    CaptureEventInput,
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
fn claim_next_extraction_task_orders_by_priority_and_age() {
    let mut conn = setup_conn();
    let observation_id = insert_task(
        &conn,
        "sess-observation",
        ExtractionTaskKind::ObservationExtract,
    )
    .expect("observation task should insert");
    let rollup_id = insert_task(&conn, "sess-rollup", ExtractionTaskKind::SessionRollup)
        .expect("rollup task should insert");

    let claimed = claim_next_extraction_task(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("task should be claimed");

    assert_eq!(claimed.id, rollup_id);
    assert_eq!(claimed.task_kind, ExtractionTaskKind::SessionRollup);
    assert_eq!(claimed.host, "codex-cli");
    assert_eq!(claimed.session_id.as_deref(), Some("sess-rollup"));

    let status = task_status(&conn, observation_id).0;
    assert_eq!(status, "pending");
}

#[test]
fn claim_next_extraction_task_preserves_ai_profile_from_capture_payload() -> Result<()> {
    let mut conn = setup_conn();
    record_captured_event(
        &conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id: "sess-profile",
            project: "/tmp/remem",
            cwd: None,
            event_type: "session_stop",
            role: None,
            tool_name: None,
            content: r#"{"session_id":"sess-profile","remem_ai_profile":"custom"}"#,
            task_kind: Some(ExtractionTaskKind::SessionRollup),
        },
    )?;

    let claimed = claim_next_extraction_task(&mut conn, "worker-a", 60)?
        .ok_or_else(|| anyhow::anyhow!("task should exist"))?;

    assert_eq!(claimed.ai_profile.as_deref(), Some("custom"));
    Ok(())
}

#[test]
fn claim_next_extraction_task_reads_ai_profile_from_large_capture_blob() -> Result<()> {
    let mut conn = setup_conn();
    let content = format!(
        r#"{{"session_id":"sess-large-profile","prefix":"{}","remem_ai_profile":"large-custom","suffix":"{}"}}"#,
        "a".repeat(10 * 1024),
        "b".repeat(10 * 1024)
    );
    record_captured_event(
        &conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id: "sess-large-profile",
            project: "/tmp/remem",
            cwd: None,
            event_type: "session_stop",
            role: None,
            tool_name: None,
            content: &content,
            task_kind: Some(ExtractionTaskKind::SessionRollup),
        },
    )?;

    let claimed = claim_next_extraction_task(&mut conn, "worker-a", 60)?
        .ok_or_else(|| anyhow::anyhow!("task should exist"))?;

    assert_eq!(claimed.ai_profile.as_deref(), Some("large-custom"));
    Ok(())
}

#[test]
fn claim_next_extraction_task_does_not_double_claim_active_task() {
    let mut conn = setup_conn();
    let task_id = insert_task(&conn, "sess-single", ExtractionTaskKind::SessionRollup)
        .expect("task should insert");

    let first = claim_next_extraction_task(&mut conn, "worker-a", 60)
        .expect("first claim should succeed")
        .expect("first claim should return task");
    let second =
        claim_next_extraction_task(&mut conn, "worker-b", 60).expect("second claim should succeed");

    assert_eq!(first.id, task_id);
    assert!(second.is_none());
}

#[test]
fn release_expired_extraction_task_leases_requeues_only_expired_tasks() {
    let mut conn = setup_conn();
    let expired_id = insert_task(&conn, "sess-expired", ExtractionTaskKind::SessionRollup)
        .expect("expired task should insert");
    let fresh_id = insert_task(&conn, "sess-fresh", ExtractionTaskKind::ObservationExtract)
        .expect("fresh task should insert");

    claim_next_extraction_task(&mut conn, "worker-expired", 60)
        .expect("expired worker claim should succeed")
        .expect("expired task should be claimed");
    claim_next_extraction_task(&mut conn, "worker-fresh", 60)
        .expect("fresh worker claim should succeed")
        .expect("fresh task should be claimed");
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "UPDATE extraction_tasks
         SET lease_expires_epoch = ?1
         WHERE id = ?2",
        params![now - 1, expired_id],
    )
    .expect("expired lease should update");

    let released = release_expired_extraction_task_leases(&conn).expect("release should succeed");

    assert_eq!(released, 1);
    assert_eq!(task_status(&conn, expired_id).0, "pending");
    assert_eq!(task_status(&conn, fresh_id).0, "processing");
}

#[test]
fn mark_extraction_task_done_clears_lease_and_advances_cursor() {
    let mut conn = setup_conn();
    let task_id = insert_task(&conn, "sess-done", ExtractionTaskKind::SessionRollup)
        .expect("task should insert");
    let task = claim_next_extraction_task(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("task should be claimed");

    mark_extraction_task_done(&conn, task.id, "worker-a", task.high_watermark_event_id)
        .expect("done should succeed");

    let (status, lease_owner, cursor, high_watermark): (
        String,
        Option<String>,
        Option<i64>,
        Option<i64>,
    ) = conn
        .query_row(
            "SELECT status, lease_owner, cursor_event_id, high_watermark_event_id
             FROM extraction_tasks WHERE id = ?1",
            params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("task should query");
    assert_eq!(status, "done");
    assert!(lease_owner.is_none());
    assert_eq!(cursor, high_watermark);
}

#[test]
fn mark_extraction_task_done_requeues_when_watermark_advanced_after_claim() {
    let mut conn = setup_conn();
    let task_id = insert_task(&conn, "sess-coalesce", ExtractionTaskKind::SessionRollup)
        .expect("task should insert");
    let task = claim_next_extraction_task(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("task should be claimed");
    let claimed_high_watermark = task.high_watermark_event_id;
    insert_task(&conn, "sess-coalesce", ExtractionTaskKind::SessionRollup)
        .expect("coalesced task should update high watermark");

    mark_extraction_task_done(&conn, task.id, "worker-a", claimed_high_watermark)
        .expect("done should succeed");

    let (status, lease_owner, cursor, high_watermark): (
        String,
        Option<String>,
        Option<i64>,
        Option<i64>,
    ) = conn
        .query_row(
            "SELECT status, lease_owner, cursor_event_id, high_watermark_event_id
             FROM extraction_tasks WHERE id = ?1",
            params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("task should query");
    assert_eq!(status, "pending");
    assert!(lease_owner.is_none());
    assert_eq!(cursor, claimed_high_watermark);
    assert!(high_watermark > cursor);
}

#[test]
fn mark_extraction_task_done_with_partial_progress_requeues_remaining_range() {
    // Models the MemoryCandidate worker returning Written { to_event_id: x }
    // with x < high_watermark: the task must requeue so the remaining
    // (x, watermark] range is claimed on the next round instead of being lost.
    let mut conn = setup_conn();
    let task_id = insert_task(&conn, "sess-partial", ExtractionTaskKind::MemoryCandidate)
        .expect("task should insert");
    insert_task(&conn, "sess-partial", ExtractionTaskKind::MemoryCandidate)
        .expect("second event should coalesce and advance the watermark");

    let task = claim_next_extraction_task(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("task should be claimed");
    assert_eq!(task.id, task_id);
    let watermark = task
        .high_watermark_event_id
        .expect("claimed task should have a watermark");
    let partial_event_id: i64 = conn
        .query_row(
            "SELECT MIN(id) FROM captured_events WHERE session_id = 'sess-partial'",
            [],
            |row| row.get(0),
        )
        .expect("first captured event id should query");
    assert!(
        partial_event_id < watermark,
        "fixture must produce partial progress: to_event_id={partial_event_id} watermark={watermark}"
    );

    mark_extraction_task_done(&conn, task.id, "worker-a", Some(partial_event_id))
        .expect("done with partial progress should succeed");

    let (status, lease_owner, cursor): (String, Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT status, lease_owner, cursor_event_id FROM extraction_tasks WHERE id = ?1",
            params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("task should query");
    assert_eq!(status, "pending", "partial progress must requeue the task");
    assert!(lease_owner.is_none());
    assert_eq!(cursor, Some(partial_event_id));

    let reclaimed = claim_next_extraction_task(&mut conn, "worker-b", 60)
        .expect("claim should succeed")
        .expect("requeued task should be claimable");
    assert_eq!(reclaimed.id, task_id);
    assert_eq!(
        reclaimed.cursor_event_id,
        Some(partial_event_id),
        "next round must resume from the written cursor"
    );
    assert_eq!(
        reclaimed.high_watermark_event_id,
        Some(watermark),
        "next round must still cover the remaining (cursor, watermark] range"
    );
}

#[test]
fn mark_extraction_task_failed_or_retry_keeps_retryable_task_visible() {
    let mut conn = setup_conn();
    let task_id = insert_task(&conn, "sess-retry", ExtractionTaskKind::SessionRollup)
        .expect("task should insert");
    claim_next_extraction_task(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("task should be claimed");

    mark_extraction_task_failed_or_retry(&conn, task_id, "worker-a", "temporary", 30)
        .expect("retry should succeed");

    let (status, attempts, next_retry, last_error) = task_status(&conn, task_id);
    assert_eq!(status, "pending");
    assert_eq!(attempts, 1);
    assert!(next_retry.is_some());
    assert_eq!(last_error.as_deref(), Some("temporary"));
}

#[test]
fn mark_extraction_task_failed_or_retry_exhausts_after_max_attempts() {
    let mut conn = setup_conn();
    let task_id = insert_task(&conn, "sess-failed", ExtractionTaskKind::SessionRollup)
        .expect("task should insert");
    conn.execute(
        "UPDATE extraction_tasks SET attempts = ?1 WHERE id = ?2",
        params![EXTRACTION_TASK_MAX_ATTEMPTS - 1, task_id],
    )
    .expect("attempt count should update");
    claim_next_extraction_task(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("task should be claimed");

    mark_extraction_task_failed_or_retry(&conn, task_id, "worker-a", "exhausted", 30)
        .expect("failure should succeed");

    let (status, attempts, next_retry, last_error) = task_status(&conn, task_id);
    assert_eq!(status, "failed");
    assert_eq!(attempts, EXTRACTION_TASK_MAX_ATTEMPTS);
    assert!(next_retry.is_none());
    assert_eq!(last_error.as_deref(), Some("exhausted"));
}

#[test]
fn mark_extraction_task_failed_records_permanent_failure_without_retry() {
    let mut conn = setup_conn();
    let task_id = insert_task(&conn, "sess-permanent", ExtractionTaskKind::SessionRollup)
        .expect("task should insert");
    claim_next_extraction_task(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("task should be claimed");

    mark_extraction_task_failed(&conn, task_id, "worker-a", "not implemented")
        .expect("permanent failure should succeed");

    let (status, attempts, next_retry, last_error) = task_status(&conn, task_id);
    assert_eq!(status, "failed");
    assert_eq!(attempts, 1);
    assert!(next_retry.is_none());
    assert_eq!(last_error.as_deref(), Some("not implemented"));
}

#[test]
fn defer_extraction_task_requeues_and_increments_attempts() {
    let mut conn = setup_conn();
    let task_id = insert_task(&conn, "sess-defer", ExtractionTaskKind::SessionRollup)
        .expect("task should insert");
    claim_next_extraction_task(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("task should be claimed");

    defer_extraction_task(&conn, task_id, "worker-a", "not implemented", 30)
        .expect("defer should succeed");

    let (status, attempts, next_retry, last_error) = task_status(&conn, task_id);
    assert_eq!(status, "pending");
    assert_eq!(attempts, 1);
    assert!(next_retry.is_some());
    assert_eq!(last_error.as_deref(), Some("not implemented"));
}

#[test]
fn defer_exhaustion_does_not_permanently_stall_session_extraction() {
    let mut conn = setup_conn();
    let task_id = insert_task(&conn, "sess-defer-stall", ExtractionTaskKind::SessionRollup)
        .expect("task should insert");
    conn.execute(
        "UPDATE extraction_tasks SET attempts = ?1 WHERE id = ?2",
        params![EXTRACTION_TASK_MAX_ATTEMPTS - 1, task_id],
    )
    .expect("attempt count should update");
    let stuck = claim_next_extraction_task(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("task should be claimed");
    let stuck_watermark = stuck.high_watermark_event_id;

    defer_extraction_task(&conn, task_id, "worker-a", "still ambiguous", 30)
        .expect("defer should exhaust");

    let ranges = list_extraction_replay_ranges(&conn, None, 10).expect("ranges should list");
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].source_task_id, task_id);
    assert_eq!(ranges[0].status, "pending");
    assert_eq!(Some(ranges[0].to_event_id), stuck_watermark);

    insert_task(&conn, "sess-defer-stall", ExtractionTaskKind::SessionRollup)
        .expect("new captured event should coalesce into the task");

    let reclaimed = claim_next_extraction_task(&mut conn, "worker-b", 60)
        .expect("claim should succeed")
        .expect("session should stay extractable after defer exhaustion");
    assert_eq!(reclaimed.id, task_id);
    assert_eq!(
        reclaimed.cursor_event_id, stuck_watermark,
        "cursor should advance past the exhausted range so new events are not blocked by the stuck region"
    );
    assert!(
        reclaimed.attempts < EXTRACTION_TASK_MAX_ATTEMPTS,
        "resurrected task needs retry budget, attempts={} fails terminally on the next defer",
        reclaimed.attempts
    );
}

#[test]
fn defer_extraction_task_exhausts_after_max_attempts() {
    let mut conn = setup_conn();
    let task_id = insert_task(
        &conn,
        "sess-defer-exhaust",
        ExtractionTaskKind::SessionRollup,
    )
    .expect("task should insert");
    conn.execute(
        "UPDATE extraction_tasks SET attempts = ?1 WHERE id = ?2",
        params![EXTRACTION_TASK_MAX_ATTEMPTS - 1, task_id],
    )
    .expect("attempt count should update");
    claim_next_extraction_task(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("task should be claimed");

    defer_extraction_task(&conn, task_id, "worker-a", "still ambiguous", 30)
        .expect("defer should exhaust");

    let (status, attempts, next_retry, last_error) = task_status(&conn, task_id);
    assert_eq!(status, "failed");
    assert_eq!(attempts, EXTRACTION_TASK_MAX_ATTEMPTS);
    assert!(next_retry.is_none());
    assert_eq!(last_error.as_deref(), Some("still ambiguous"));
}

#[test]
fn enqueue_followup_revives_exhausted_task_with_fresh_retry_budget() {
    let mut conn = setup_conn();
    let task_id = insert_task(
        &conn,
        "sess-followup-revive",
        ExtractionTaskKind::MemoryCandidate,
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
    let stuck_watermark = claimed
        .high_watermark_event_id
        .expect("claimed task should have a watermark");

    defer_extraction_task(&conn, task_id, "worker-a", "still ambiguous", 30)
        .expect("defer should exhaust");

    let followup_id = enqueue_followup_extraction_task(
        &conn,
        &claimed,
        ExtractionTaskKind::MemoryCandidate,
        stuck_watermark + 1,
    )
    .expect("followup should coalesce into the exhausted task");

    assert_eq!(followup_id, task_id);
    let (status, attempts, next_retry, _) = task_status(&conn, task_id);
    assert_eq!(status, "pending");
    assert_eq!(attempts, 0, "revived task needs a fresh retry budget");
    assert!(next_retry.is_none());
}

#[test]
fn enqueue_bounded_followup_revives_failed_task_from_original_range_start() {
    let mut conn = setup_conn();
    insert_task(
        &conn,
        "sess-bounded-followup-revive",
        ExtractionTaskKind::SessionRollup,
    )
    .expect("source task should insert");
    let source = claim_next_extraction_task(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("source task should be claimed");
    let high_watermark = source
        .high_watermark_event_id
        .expect("source task should have a watermark");
    let bounded_id = enqueue_bounded_followup_extraction_task(
        &conn,
        &source,
        ExtractionTaskKind::UserContextCandidate,
        0,
        high_watermark,
    )
    .expect("bounded followup should enqueue");
    conn.execute(
        "UPDATE extraction_tasks SET attempts = ?1 WHERE id = ?2",
        params![EXTRACTION_TASK_MAX_ATTEMPTS - 1, bounded_id],
    )
    .expect("bounded task attempts should update");
    let bounded = claim_next_extraction_task(&mut conn, "worker-b", 60)
        .expect("claim should succeed")
        .expect("bounded task should be claimable");
    assert_eq!(bounded.id, bounded_id);
    defer_claimed_extraction_task(&conn, &bounded, "worker-b", "exhausted", 30)
        .expect("bounded task should exhaust");
    let ranges = list_extraction_replay_ranges(&conn, None, 10).expect("ranges should list");
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].status, "pending");
    assert_eq!(ranges[0].source_task_id, bounded_id);

    let revived_id = enqueue_bounded_followup_extraction_task(
        &conn,
        &source,
        ExtractionTaskKind::UserContextCandidate,
        0,
        high_watermark,
    )
    .expect("same bounded followup should revive");

    assert_eq!(revived_id, bounded_id);
    let (status, attempts, cursor, next_retry, lease_owner, last_error, replay_range_id): (
        String,
        i64,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<i64>,
    ) = conn
        .query_row(
            "SELECT status, attempts, cursor_event_id, next_retry_epoch, lease_owner, last_error,
                    replay_range_id
             FROM extraction_tasks WHERE id = ?1",
            params![bounded_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .expect("revived bounded task should query");
    assert_eq!(status, "pending");
    assert_eq!(attempts, 0);
    assert_eq!(cursor, Some(0), "revival must retry the full bounded range");
    assert!(next_retry.is_none());
    assert!(lease_owner.is_none());
    assert!(last_error.is_none());
    assert_eq!(replay_range_id, Some(ranges[0].id));
    let ranges = list_extraction_replay_ranges(&conn, None, 10).expect("ranges should list");
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].status, "requeued");

    let revived = claim_next_extraction_task(&mut conn, "worker-c", 60)
        .expect("claim should succeed")
        .expect("revived bounded task should be claimable");
    assert_eq!(revived.id, bounded_id);
    mark_extraction_task_done(
        &conn,
        revived.id,
        "worker-c",
        revived.high_watermark_event_id,
    )
    .expect("revived bounded task should finish");
    assert!(
        list_extraction_replay_ranges(&conn, None, 10)
            .expect("ranges should list")
            .is_empty(),
        "successful revived bounded retry must clear the recorded replay range"
    );
}

#[test]
fn exhaustion_records_only_claimed_range_when_watermark_advances() {
    let mut conn = setup_conn();
    let task_id = insert_task(
        &conn,
        "sess-claim-watermark",
        ExtractionTaskKind::SessionRollup,
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
    let claimed_watermark = claimed
        .high_watermark_event_id
        .expect("claimed task should have a watermark");

    insert_task(
        &conn,
        "sess-claim-watermark",
        ExtractionTaskKind::SessionRollup,
    )
    .expect("new captured event should coalesce while the task is leased");
    defer_claimed_extraction_task(&conn, &claimed, "worker-a", "still ambiguous", 30)
        .expect("defer should exhaust the claimed range");

    let ranges = list_extraction_replay_ranges(&conn, None, 10).expect("ranges should list");
    assert_eq!(ranges.len(), 1);
    assert_eq!(
        ranges[0].from_event_id,
        claimed.cursor_event_id.unwrap_or(0) + 1
    );
    assert_eq!(ranges[0].to_event_id, claimed_watermark);

    let (status, attempts, next_retry, _) = task_status(&conn, task_id);
    assert_eq!(status, "pending");
    assert_eq!(attempts, 0);
    assert!(next_retry.is_none());
    let reclaimed = claim_next_extraction_task(&mut conn, "worker-b", 60)
        .expect("claim should succeed")
        .expect("later range should be claimable");
    assert_eq!(reclaimed.id, task_id);
    assert_eq!(reclaimed.cursor_event_id, Some(claimed_watermark));
    assert!(
        reclaimed.high_watermark_event_id > reclaimed.cursor_event_id,
        "later event must remain available to the primary task"
    );
}

#[test]
fn retry_extraction_replay_range_creates_bounded_replay_task() {
    let mut conn = setup_conn();
    let task_id = insert_task(&conn, "sess-replay", ExtractionTaskKind::ObservationExtract)
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

    let retried = retry_extraction_replay_ranges(&conn, None, 10).expect("retry should enqueue");
    assert_eq!(retried, 1);
    let listed = list_extraction_replay_ranges(&conn, None, 10).expect("ranges should list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, "requeued");

    let replay = claim_next_extraction_task(&mut conn, "worker-b", 60)
        .expect("claim should succeed")
        .expect("replay task should be claimable");
    assert_ne!(replay.id, task_id);
    assert_eq!(replay.replay_range_id, Some(range.id));
    assert_eq!(replay.cursor_event_id, Some(range.from_event_id - 1));
    assert_eq!(replay.high_watermark_event_id, Some(range.to_event_id));
}

#[test]
fn replayed_range_clears_terminal_source_failure() {
    let mut conn = setup_conn();
    let task_id = insert_task(
        &conn,
        "sess-replay-source-clear",
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
    assert_eq!(task_status(&conn, task_id).0, "failed");

    retry_extraction_replay_ranges(&conn, None, 10).expect("retry should enqueue");
    let replay = claim_next_extraction_task(&mut conn, "worker-b", 60)
        .expect("claim should succeed")
        .expect("replay task should be claimable");
    mark_extraction_task_done(&conn, replay.id, "worker-b", replay.high_watermark_event_id)
        .expect("replay task should finish");

    assert_eq!(
        task_status(&conn, task_id).0,
        "done",
        "successful replay should clear the terminal source task failure"
    );
    assert!(
        list_extraction_replay_ranges(&conn, None, 10)
            .expect("ranges should list")
            .is_empty(),
        "replayed ranges should leave the operational queue"
    );
}

#[test]
fn replay_followup_stays_scoped_to_replay_range() {
    let mut conn = setup_conn();
    let task_id = insert_task(
        &conn,
        "sess-replay-followup",
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
    retry_extraction_replay_ranges(&conn, None, 10).expect("retry should enqueue");
    let replay = claim_next_extraction_task(&mut conn, "worker-b", 60)
        .expect("claim should succeed")
        .expect("replay task should be claimable");

    let followup_id = enqueue_followup_extraction_task(
        &conn,
        &replay,
        ExtractionTaskKind::MemoryCandidate,
        replay
            .high_watermark_event_id
            .expect("replay should have watermark"),
    )
    .expect("followup should enqueue");
    let (cursor, high_watermark, replay_range_id): (Option<i64>, Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT cursor_event_id, high_watermark_event_id, replay_range_id
             FROM extraction_tasks WHERE id = ?1",
            params![followup_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("followup should query");
    assert_eq!(cursor, replay.cursor_event_id);
    assert_eq!(high_watermark, replay.high_watermark_event_id);
    assert_eq!(replay_range_id, replay.replay_range_id);

    mark_extraction_task_done(&conn, replay.id, "worker-b", replay.high_watermark_event_id)
        .expect("replay task should finish");
    let ranges = list_extraction_replay_ranges(&conn, None, 10).expect("ranges should list");
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].status, "requeued");

    let followup = claim_next_extraction_task(&mut conn, "worker-c", 60)
        .expect("claim should succeed")
        .expect("followup task should be claimable");
    assert_eq!(followup.id, followup_id);
    mark_extraction_task_done(
        &conn,
        followup.id,
        "worker-c",
        followup.high_watermark_event_id,
    )
    .expect("followup should finish");
    assert!(
        list_extraction_replay_ranges(&conn, None, 10)
            .expect("ranges should list")
            .is_empty(),
        "range should disappear from operational list only after the replay chain finishes"
    );
}

#[test]
fn replay_range_stays_failed_when_followup_fails_before_parent_done() {
    let mut conn = setup_conn();
    let task_id = insert_task(
        &conn,
        "sess-replay-followup-failure",
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
    retry_extraction_replay_ranges(&conn, None, 10).expect("retry should enqueue");
    let replay = claim_next_extraction_task(&mut conn, "worker-b", 60)
        .expect("claim should succeed")
        .expect("replay task should be claimable");
    let followup_id = enqueue_followup_extraction_task(
        &conn,
        &replay,
        ExtractionTaskKind::MemoryCandidate,
        replay
            .high_watermark_event_id
            .expect("replay should have watermark"),
    )
    .expect("followup should enqueue");

    let followup = claim_next_extraction_task(&mut conn, "worker-c", 60)
        .expect("claim should succeed")
        .expect("followup task should be claimable");
    assert_eq!(followup.id, followup_id);
    mark_extraction_task_failed(&conn, followup.id, "worker-c", "followup failed")
        .expect("followup failure should mark replay range failed");
    let ranges = list_extraction_replay_ranges(&conn, None, 10).expect("ranges should list");
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].status, "failed");
    assert_eq!(
        count_retryable_extraction_replay_ranges(&conn, None, 10).expect("count should succeed"),
        0
    );
    assert_eq!(
        retry_extraction_replay_ranges(&conn, None, 10).expect("retry should skip active siblings"),
        0
    );
    assert_eq!(
        quarantine_extraction_replay_ranges(&conn, None, 10)
            .expect("quarantine should skip active siblings"),
        0
    );
    mark_extraction_task_done(&conn, replay.id, "worker-b", replay.high_watermark_event_id)
        .expect("parent replay task should finish");
    assert_eq!(
        count_retryable_extraction_replay_ranges(&conn, None, 10).expect("count should succeed"),
        1
    );
    assert_eq!(
        quarantine_extraction_replay_ranges(&conn, None, 10).expect("quarantine should succeed"),
        1
    );
    assert_eq!(task_status(&conn, followup_id).0, "done");
    assert_eq!(task_status(&conn, task_id).0, "done");
    let ranges = list_extraction_replay_ranges(&conn, None, 10).expect("ranges should list");
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].status, "quarantined");
}

#[test]
fn quarantine_extraction_replay_ranges_removes_retryable_ranges() {
    let mut conn = setup_conn();
    let task_id = insert_task(&conn, "sess-quarantine", ExtractionTaskKind::SessionRollup)
        .expect("task should insert");
    conn.execute(
        "UPDATE extraction_tasks SET attempts = ?1 WHERE id = ?2",
        params![EXTRACTION_TASK_MAX_ATTEMPTS - 1, task_id],
    )
    .expect("attempt count should update");
    let claimed = claim_next_extraction_task(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("task should be claimed");
    defer_claimed_extraction_task(&conn, &claimed, "worker-a", "still ambiguous", 30)
        .expect("defer should exhaust");
    assert_eq!(task_status(&conn, task_id).0, "failed");

    assert_eq!(
        count_retryable_extraction_replay_ranges(&conn, None, 10).expect("count should succeed"),
        1
    );
    let quarantined =
        quarantine_extraction_replay_ranges(&conn, None, 10).expect("quarantine should succeed");
    assert_eq!(quarantined, 1);
    assert_eq!(
        count_retryable_extraction_replay_ranges(&conn, None, 10).expect("count should succeed"),
        0
    );
    let ranges = list_extraction_replay_ranges(&conn, None, 10).expect("ranges should list");
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].status, "quarantined");
    assert_eq!(task_status(&conn, task_id).0, "done");
}
