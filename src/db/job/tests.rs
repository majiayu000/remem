use rusqlite::{params, types::Value, Connection};

use super::{
    claim_next_job, enqueue_job, mark_job_done, mark_job_exhausted, mark_job_failed,
    mark_job_failed_or_retry, maybe_enqueue_dream_job, release_expired_job_leases,
    DreamEnqueueDecision, ExpiredJobLeaseOutcome, JobIdentityKind, JobTransitionOutcome, JobType,
};
use crate::migrate::MIGRATIONS;

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    for migration in MIGRATIONS {
        conn.execute_batch(migration.sql)
            .expect("schema migration should load");
    }
    conn
}

fn expect_enqueued(decision: DreamEnqueueDecision) -> i64 {
    match decision {
        DreamEnqueueDecision::Enqueued(id) => id,
        other => panic!("expected a newly enqueued Dream job, got {other:?}"),
    }
}

fn enqueue_and_claim(conn: &mut Connection, project: &str, owner: &str) -> i64 {
    let job_id = enqueue_job(
        conn,
        "codex-cli",
        JobType::Observation,
        project,
        Some("session"),
        "{\"fixture\":true}",
        100,
    )
    .expect("fixture job should enqueue");
    let claimed = claim_next_job(conn, owner, 60)
        .expect("fixture claim should succeed")
        .expect("fixture job should be claimable");
    assert_eq!(claimed.id, job_id);
    job_id
}

fn job_snapshot(conn: &Connection, job_id: i64) -> Vec<Value> {
    conn.query_row(
        "SELECT id, host, job_type, project, session_id, payload_json, state,
                priority, attempt_count, max_attempts, lease_owner,
                lease_expires_epoch, next_retry_epoch, last_error,
                created_at_epoch, updated_at_epoch, failure_class,
                failed_at_epoch, archived_at_epoch
         FROM jobs WHERE id = ?1",
        params![job_id],
        |row| (0..19).map(|column| row.get(column)).collect(),
    )
    .expect("job snapshot should load")
}

#[test]
fn lease_owned_job_transitions_require_current_unexpired_lease() {
    let mut conn = setup_conn();

    let done_id = enqueue_and_claim(&mut conn, "done", "worker-a");
    assert!(mark_job_done(&conn, done_id, "worker-b").is_err());
    mark_job_done(&conn, done_id, "worker-a").expect("current owner should complete the job");
    assert!(mark_job_done(&conn, done_id, "worker-a").is_err());

    let retry_id = enqueue_and_claim(&mut conn, "retry", "worker-a");
    conn.execute(
        "UPDATE jobs SET lease_expires_epoch = ?2 WHERE id = ?1",
        params![retry_id, chrono::Utc::now().timestamp() - 1],
    )
    .expect("retry lease should expire");
    assert!(mark_job_failed(&conn, retry_id, "worker-a", "boom", 30).is_err());

    let exhausted_id = enqueue_and_claim(&mut conn, "exhausted", "worker-a");
    conn.execute(
        "UPDATE jobs SET lease_owner = 'worker-b' WHERE id = ?1",
        params![exhausted_id],
    )
    .expect("lease should be reassigned");
    assert!(mark_job_exhausted(&conn, exhausted_id, "worker-a").is_err());

    let permanent_id = enqueue_and_claim(&mut conn, "permanent", "worker-a");
    conn.execute(
        "UPDATE jobs SET lease_expires_epoch = ?2 WHERE id = ?1",
        params![permanent_id, chrono::Utc::now().timestamp() - 1],
    )
    .expect("permanent failure lease should expire");
    assert!(
        mark_job_failed_or_retry(&conn, permanent_id, "worker-a", "not implemented", 30,).is_err()
    );
}

#[test]
fn rejected_job_transition_preserves_every_persisted_field() {
    let mut conn = setup_conn();
    let job_id = enqueue_and_claim(&mut conn, "preserve", "worker-a");
    conn.execute(
        "UPDATE jobs
         SET priority = 7, attempt_count = 2, max_attempts = 9,
             next_retry_epoch = 123, last_error = 'original',
             failure_class = 'transient', failed_at_epoch = 456,
             archived_at_epoch = 789
         WHERE id = ?1",
        params![job_id],
    )
    .expect("fixture evidence should update");
    let before = job_snapshot(&conn, job_id);

    let error = mark_job_failed_or_retry(&conn, job_id, "worker-b", "replacement", 30)
        .expect_err("wrong owner must reject the transition");

    assert!(error.to_string().contains("worker-b"));
    assert_eq!(job_snapshot(&conn, job_id), before);
}

#[test]
fn job_transition_error_reports_expected_and_current_lease() {
    let mut conn = setup_conn();
    let job_id = enqueue_and_claim(&mut conn, "diagnostic", "worker-current");
    conn.execute(
        "UPDATE jobs SET lease_expires_epoch = 1700000000 WHERE id = ?1",
        params![job_id],
    )
    .expect("fixture lease should update");

    let error = mark_job_done(&conn, job_id, "worker-expected")
        .expect_err("mismatched lease owner must be diagnosed")
        .to_string();
    assert!(error.contains(&format!("job_id={job_id}")));
    assert!(error.contains("expected_owner=worker-expected"));
    assert!(error.contains("current_state=processing"));
    assert!(error.contains("current_owner=worker-current"));
    assert!(error.contains("lease_expires_epoch=1700000000"));

    let missing = mark_job_done(&conn, i64::MAX, "worker-expected")
        .expect_err("missing job must be diagnosed")
        .to_string();
    assert!(missing.contains("current=missing"));
}

fn compile_rules_with_successor(conn: &Connection, project: &str, expiry: i64) -> (i64, i64) {
    let source = enqueue_job(
        conn,
        "worker",
        JobType::CompileRules,
        project,
        None,
        "{\"source\":true}",
        50,
    )
    .expect("CompileRules source should enqueue");
    conn.execute(
        "UPDATE jobs SET state = 'processing', lease_owner = 'worker-a',
             lease_expires_epoch = ?2, attempt_count = 2,
             last_error = 'existing failure' WHERE id = ?1",
        params![source, expiry],
    )
    .expect("CompileRules source should enter processing");
    let successor = enqueue_job(
        conn,
        "worker",
        JobType::CompileRules,
        project,
        None,
        "{\"successor\":true}",
        200,
    )
    .expect("CompileRules successor should enqueue");
    (source, successor)
}

#[test]
fn compile_rules_retry_collision_coalesces_to_pending_successor() {
    let conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let (source, successor) = compile_rules_with_successor(&conn, "retry-collision", now + 60);
    let source_before = job_snapshot(&conn, source);
    let successor_before = job_snapshot(&conn, successor);
    conn.execute_batch(&format!(
        "CREATE TRIGGER fail_compile_source BEFORE UPDATE OF state ON jobs
         WHEN OLD.id = {source} AND NEW.state = 'failed'
         BEGIN SELECT RAISE(ABORT, 'injected source failure'); END;"
    ))
    .expect("rollback trigger should install");
    assert!(mark_job_failed_or_retry(&conn, source, "worker-a", "boom", 30).is_err());
    assert_eq!(job_snapshot(&conn, source), source_before);
    assert_eq!(job_snapshot(&conn, successor), successor_before);
    conn.execute_batch("DROP TRIGGER fail_compile_source")
        .expect("rollback trigger should drop");

    let outcome = mark_job_failed_or_retry(&conn, source, "worker-a", "boom", 30)
        .expect("retry collision should coalesce");
    assert_eq!(
        outcome,
        JobTransitionOutcome::Coalesced {
            source_id: source,
            canonical_id: successor,
            identity_kind: JobIdentityKind::CompileRules,
        }
    );
    let source_row = job_snapshot(&conn, source);
    assert_eq!(source_row[6], Value::Text("failed".into()));
    assert_eq!(source_row[8], Value::Integer(3));
    assert_eq!(source_row[12], Value::Integer(0));
    assert!(matches!(&source_row[13], Value::Text(error) if error.starts_with("boom ")));
    assert_eq!(source_row[16], Value::Text("permanent".into()));
    let successor_row = job_snapshot(&conn, successor);
    assert_eq!(successor_row[6], Value::Text("pending".into()));
    assert_eq!(successor_row[7], Value::Integer(50));
    assert!(matches!(successor_row[12], Value::Integer(epoch) if epoch >= now + 29));
    assert_eq!(successor_row[5], Value::Text("{\"successor\":true}".into()));
}

#[test]
fn compile_rules_retry_collision_preserves_original_error_with_bounded_marker() {
    let conn = setup_conn();
    let (source, successor) =
        compile_rules_with_successor(&conn, "bounded-error", chrono::Utc::now().timestamp() + 60);
    let error = "e".repeat(2_000);
    mark_job_failed_or_retry(&conn, source, "worker-a", &error, 30)
        .expect("retry collision should coalesce");
    let persisted: String = conn
        .query_row(
            "SELECT last_error FROM jobs WHERE id = ?1",
            params![source],
            |row| row.get(0),
        )
        .expect("bounded error should load");
    assert!(persisted.len() <= 2_000);
    assert!(persisted.starts_with('e'));
    assert!(persisted.ends_with(&format!(
        "[compile_rules_retry_coalesced_to_successor id={successor}]"
    )));
}

fn expired_release_fixture() -> (Connection, i64, i64, i64) {
    let conn = setup_conn();
    let expired = chrono::Utc::now().timestamp() - 10;
    let (source, successor) = compile_rules_with_successor(&conn, "expired", expired);
    let ordinary = enqueue_job(
        &conn,
        "codex-cli",
        JobType::Observation,
        "ordinary-expired",
        Some("session"),
        "{}",
        100,
    )
    .expect("ordinary job should enqueue");
    conn.execute(
        "UPDATE jobs SET state = 'processing', lease_owner = 'worker-b',
             lease_expires_epoch = ?2 WHERE id = ?1",
        params![ordinary, expired + 1],
    )
    .expect("ordinary lease should expire");
    (conn, source, successor, ordinary)
}

#[test]
fn release_expired_compile_rules_collision_preserves_unrelated_job_progress() {
    let (conn, source, successor, ordinary) = expired_release_fixture();
    let batch = release_expired_job_leases(&conn).expect("expired batch should recover");
    assert_eq!((batch.requeued, batch.coalesced), (1, 1));
    let source_row = job_snapshot(&conn, source);
    assert_eq!(source_row[6], Value::Text("failed".into()));
    assert_eq!(source_row[8], Value::Integer(2));
    assert_eq!(source_row[9], Value::Integer(6));
    assert!(
        matches!(&source_row[13], Value::Text(error) if error.starts_with("existing failure ") && error.contains("coalesced_to_successor"))
    );
    assert_eq!(
        job_snapshot(&conn, successor)[6],
        Value::Text("pending".into())
    );
    assert_eq!(
        job_snapshot(&conn, ordinary)[6],
        Value::Text("pending".into())
    );
}

#[test]
fn release_expired_job_leases_reports_requeued_and_coalesced_outcomes() {
    let (conn, source, successor, ordinary) = expired_release_fixture();
    let batch = release_expired_job_leases(&conn).expect("expired batch should recover");
    assert_eq!(
        batch.outcomes,
        vec![
            ExpiredJobLeaseOutcome::Coalesced {
                source_id: source,
                canonical_id: successor,
                identity_kind: JobIdentityKind::CompileRules,
            },
            ExpiredJobLeaseOutcome::Requeued {
                source_id: ordinary,
                identity_kind: JobIdentityKind::Ordinary,
            },
        ]
    );
}

#[test]
fn enqueue_job_dedups_inflight_job() {
    let conn = setup_conn();
    let first = enqueue_job(
        &conn,
        "codex-cli",
        JobType::Summary,
        "alpha",
        Some("s1"),
        "{}",
        100,
    )
    .expect("first enqueue should succeed");
    let second = enqueue_job(
        &conn,
        "codex-cli",
        JobType::Summary,
        "alpha",
        Some("s1"),
        "{}",
        100,
    )
    .expect("second enqueue should dedup");

    assert_eq!(first, second);
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
        .expect("job count should load");
    assert_eq!(count, 1);
}

#[test]
fn enqueue_job_dedupe_includes_host() {
    let conn = setup_conn();
    let codex = enqueue_job(
        &conn,
        "codex-cli",
        JobType::Summary,
        "alpha",
        Some("s1"),
        "{}",
        100,
    )
    .expect("codex enqueue should succeed");
    let claude = enqueue_job(
        &conn,
        "claude-code",
        JobType::Summary,
        "alpha",
        Some("s1"),
        "{}",
        100,
    )
    .expect("claude enqueue should succeed");
    let codex_again = enqueue_job(
        &conn,
        "codex-cli",
        JobType::Summary,
        "alpha",
        Some("s1"),
        "{}",
        100,
    )
    .expect("codex duplicate should dedup");

    assert_ne!(codex, claude);
    assert_eq!(codex, codex_again);
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
        .expect("job count should load");
    assert_eq!(count, 2);
}

#[test]
fn compile_rules_enqueue_keeps_one_pending_successor_while_processing() {
    let conn = setup_conn();
    let processing = enqueue_job(
        &conn,
        "worker",
        JobType::CompileRules,
        "alpha",
        None,
        "{}",
        100,
    )
    .expect("first compile enqueue should succeed");
    conn.execute(
        "UPDATE jobs SET state = 'processing' WHERE id = ?1",
        params![processing],
    )
    .expect("compile job should enter processing");

    let successor = enqueue_job(
        &conn,
        "worker",
        JobType::CompileRules,
        "alpha",
        None,
        "{}",
        100,
    )
    .expect("lifecycle update should enqueue a successor");
    let duplicate = enqueue_job(
        &conn,
        "worker",
        JobType::CompileRules,
        "alpha",
        None,
        "{}",
        100,
    )
    .expect("pending successor should deduplicate");

    assert_ne!(successor, processing);
    assert_eq!(duplicate, successor);
    let states: (i64, i64) = conn
        .query_row(
            "SELECT SUM(state = 'processing'), SUM(state = 'pending')
             FROM jobs WHERE job_type = 'compile_rules' AND project = 'alpha'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("compile job states should load");
    assert_eq!(states, (1, 1));
}

#[test]
fn claim_next_job_picks_highest_priority_ready_job() {
    let mut conn = setup_conn();
    let low = enqueue_job(
        &conn,
        "codex-cli",
        JobType::Summary,
        "alpha",
        Some("s1"),
        "{}",
        200,
    )
    .expect("low priority enqueue should succeed");
    let high = enqueue_job(
        &conn,
        "codex-cli",
        JobType::Observation,
        "alpha",
        Some("s2"),
        "{}",
        50,
    )
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
    let job_id = enqueue_job(
        &conn,
        "codex-cli",
        JobType::Summary,
        "alpha",
        Some("s1"),
        "{}",
        100,
    )
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
fn mark_job_failed_or_retry_fails_permanent_error_without_retry() {
    let mut conn = setup_conn();
    let job_id = enqueue_job(
        &conn,
        "codex-cli",
        JobType::Summary,
        "alpha",
        Some("s1"),
        "{}",
        100,
    )
    .expect("job enqueue should succeed");
    let claimed = claim_next_job(&mut conn, "worker-a", 60)
        .expect("claim should succeed")
        .expect("job should be claimed");

    mark_job_failed_or_retry(&conn, claimed.id, "worker-a", "not implemented", 30)
        .expect("permanent failure should succeed");

    let row = conn
        .query_row(
            "SELECT state, attempt_count, lease_owner, next_retry_epoch, last_error, failure_class
             FROM jobs WHERE id = ?1",
            params![job_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .expect("job row should load");
    assert_eq!(row.0, "failed");
    assert_eq!(row.1, 1);
    assert_eq!(row.2, None);
    assert_eq!(row.3, 0);
    assert_eq!(row.4.as_deref(), Some("not implemented"));
    assert_eq!(row.5.as_deref(), Some("permanent"));
}

#[test]
fn mark_job_failed_or_retry_marks_failed_when_exhausted() {
    let mut conn = setup_conn();
    let job_id = enqueue_job(
        &conn,
        "codex-cli",
        JobType::Summary,
        "alpha",
        Some("s1"),
        "{}",
        100,
    )
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

#[test]
fn maybe_enqueue_dream_job_dedups_inflight_job() {
    let conn = setup_conn();
    let first = expect_enqueued(
        maybe_enqueue_dream_job(&conn, "codex-cli", "alpha", "{}", 300, 3600)
            .expect("first dream enqueue should succeed"),
    );
    let second = maybe_enqueue_dream_job(&conn, "codex-cli", "alpha", "{}", 300, 3600)
        .expect("second dream enqueue should succeed");

    assert_eq!(second, DreamEnqueueDecision::CoalescedInflight(first));
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
        .expect("job count should load");
    assert_eq!(count, 1);
    let state: String = conn
        .query_row(
            "SELECT state FROM jobs WHERE id = ?1",
            params![first],
            |row| row.get(0),
        )
        .expect("state should load");
    assert_eq!(state, "pending");
}

#[test]
fn maybe_enqueue_dream_job_dedups_inflight_across_hosts_for_project() {
    let conn = setup_conn();
    let first = expect_enqueued(
        maybe_enqueue_dream_job(&conn, "codex-cli", "alpha", "{}", 300, 3600)
            .expect("first dream enqueue should succeed"),
    );
    let second = maybe_enqueue_dream_job(&conn, "claude-code", "alpha", "{}", 300, 3600)
        .expect("second dream enqueue should succeed");

    assert_eq!(second, DreamEnqueueDecision::CoalescedInflight(first));
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
        .expect("job count should load");
    assert_eq!(count, 1);
}

#[test]
fn maybe_enqueue_dream_job_upgrades_pending_payload_for_profile_override() {
    let conn = setup_conn();
    let first = expect_enqueued(
        maybe_enqueue_dream_job(&conn, "codex-cli", "alpha", "{}", 300, 3600)
            .expect("first dream enqueue should succeed"),
    );
    let profile_payload = r#"{"remem_ai_profile":"quality"}"#;

    let second = maybe_enqueue_dream_job(&conn, "claude-code", "alpha", profile_payload, 100, 3600)
        .expect("profile dream enqueue should succeed");

    assert_eq!(second, DreamEnqueueDecision::CoalescedInflight(first));
    let row: (String, String, i64) = conn
        .query_row(
            "SELECT host, payload_json, priority FROM jobs WHERE id = ?1",
            params![first],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("job row should load");
    assert_eq!(row.0, "claude-code");
    assert_eq!(row.1, profile_payload);
    assert_eq!(row.2, 100);
}

#[test]
fn maybe_enqueue_dream_job_skips_recent_done_job() {
    let conn = setup_conn();
    let first = expect_enqueued(
        maybe_enqueue_dream_job(&conn, "codex-cli", "alpha", "{}", 300, 3600)
            .expect("first dream enqueue should succeed"),
    );
    conn.execute(
        "UPDATE jobs SET state = 'done' WHERE id = ?1",
        params![first],
    )
    .expect("job state should update");

    let second = maybe_enqueue_dream_job(&conn, "codex-cli", "alpha", "{}", 300, 3600)
        .expect("second dream enqueue should succeed");

    assert_eq!(second, DreamEnqueueDecision::SuppressedRecentDone(first));
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
        .expect("job count should load");
    assert_eq!(count, 1);
}

#[test]
fn maybe_enqueue_dream_job_skips_recent_done_across_hosts_for_same_profile() {
    let conn = setup_conn();
    let first = expect_enqueued(
        maybe_enqueue_dream_job(&conn, "codex-cli", "alpha", "{}", 300, 3600)
            .expect("first dream enqueue should succeed"),
    );
    conn.execute(
        "UPDATE jobs SET state = 'done' WHERE id = ?1",
        params![first],
    )
    .expect("job state should update");

    let second = maybe_enqueue_dream_job(&conn, "claude-code", "alpha", "{}", 300, 3600)
        .expect("second dream enqueue should succeed");

    assert_eq!(second, DreamEnqueueDecision::SuppressedRecentDone(first));
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
        .expect("job count should load");
    assert_eq!(count, 1);
}

#[test]
fn maybe_enqueue_dream_job_skips_recent_done_with_different_profile_for_project_cooldown() {
    let conn = setup_conn();
    let first = expect_enqueued(
        maybe_enqueue_dream_job(&conn, "codex-cli", "alpha", "{}", 300, 3600)
            .expect("first dream enqueue should succeed"),
    );
    conn.execute(
        "UPDATE jobs SET state = 'done' WHERE id = ?1",
        params![first],
    )
    .expect("job state should update");

    let second = maybe_enqueue_dream_job(
        &conn,
        "claude-code",
        "alpha",
        r#"{"remem_ai_profile":"quality"}"#,
        300,
        3600,
    )
    .expect("profile dream enqueue should succeed");

    assert_eq!(second, DreamEnqueueDecision::SuppressedRecentDone(first));
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
        .expect("job count should load");
    assert_eq!(count, 1);
}

#[test]
fn maybe_enqueue_dream_job_allows_old_done_job() {
    let conn = setup_conn();
    let first = expect_enqueued(
        maybe_enqueue_dream_job(&conn, "codex-cli", "alpha", "{}", 300, 3600)
            .expect("first dream enqueue should succeed"),
    );
    conn.execute(
        "UPDATE jobs SET state = 'done', updated_at_epoch = ?2 WHERE id = ?1",
        params![first, chrono::Utc::now().timestamp() - 7200],
    )
    .expect("job state should update");

    let second = expect_enqueued(
        maybe_enqueue_dream_job(&conn, "codex-cli", "alpha", "{}", 300, 3600)
            .expect("second dream enqueue should succeed"),
    );

    assert_ne!(first, second);
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
        .expect("job count should load");
    assert_eq!(count, 2);
}

#[test]
fn maybe_enqueue_dream_job_allows_failed_job_retry_visibility() {
    let conn = setup_conn();
    let first = expect_enqueued(
        maybe_enqueue_dream_job(&conn, "codex-cli", "alpha", "{}", 300, 3600)
            .expect("first dream enqueue should succeed"),
    );
    conn.execute(
        "UPDATE jobs SET state = 'failed' WHERE id = ?1",
        params![first],
    )
    .expect("job state should update");

    let second = expect_enqueued(
        maybe_enqueue_dream_job(&conn, "codex-cli", "alpha", "{}", 300, 3600)
            .expect("second dream enqueue should succeed"),
    );

    assert_ne!(first, second);
}
