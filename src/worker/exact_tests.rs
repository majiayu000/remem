use rusqlite::params;

use crate::db::{self, test_support::ScopedTestDataDir};

use super::{lock, run_exact_replay};

fn archived_quarantined_range(
    conn: &mut rusqlite::Connection,
    session_id: &str,
    task_kind: db::ExtractionTaskKind,
) -> anyhow::Result<i64> {
    let outcome = db::record_captured_event(
        conn,
        &db::CaptureEventInput {
            host: "codex-cli",
            session_id,
            project: "/tmp/remem-exact",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: session_id,
            task_kind: Some(task_kind),
        },
    )?;
    let task_id = outcome
        .extraction_task_id
        .ok_or_else(|| anyhow::anyhow!("expected extraction task"))?;
    conn.execute(
        "UPDATE extraction_tasks SET attempts = ?1 WHERE id = ?2",
        params![db::EXTRACTION_TASK_MAX_ATTEMPTS - 1, task_id],
    )?;
    let task = db::claim_extraction_task_by_id(conn, task_id, "fixture-worker", 60)?
        .ok_or_else(|| anyhow::anyhow!("fixture task should claim"))?;
    db::defer_claimed_extraction_task(conn, &task, "fixture-worker", "fixture exhausted", 1)?;
    let range_id = db::list_extraction_replay_ranges(conn, None, 20)?
        .into_iter()
        .find(|range| range.source_task_id == task_id)
        .map(|range| range.id)
        .ok_or_else(|| anyhow::anyhow!("expected replay range"))?;
    db::quarantine_extraction_replay_range(conn, range_id)?;
    conn.execute(
        "UPDATE extraction_replay_ranges SET archived_at_epoch = 1 WHERE id = ?1",
        params![range_id],
    )?;
    Ok(range_id)
}

fn range_state(
    conn: &rusqlite::Connection,
    range_id: i64,
) -> anyhow::Result<(String, Option<i64>, Option<i64>)> {
    conn.query_row(
        "SELECT status, archived_at_epoch, replay_task_id
         FROM extraction_replay_ranges WHERE id = ?1",
        params![range_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .map_err(Into::into)
}

#[tokio::test]
async fn worker_exact_range_locks_before_requeue_and_processes_only_target() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-exact-range");
    crate::runtime_config::init_config()?;
    let mut conn = db::open_db()?;
    let target_id = archived_quarantined_range(
        &mut conn,
        "sess-exact-target",
        db::ExtractionTaskKind::RuleCandidate,
    )?;
    let sibling_id = archived_quarantined_range(
        &mut conn,
        "sess-exact-sibling",
        db::ExtractionTaskKind::RuleCandidate,
    )?;

    let lock_guard = lock::acquire_worker_singleton()?
        .ok_or_else(|| anyhow::anyhow!("fixture lock should acquire"))?;
    let error = run_exact_replay(target_id, true, true, "codex")
        .await
        .expect_err("held singleton must reject exact recovery");
    assert!(error.to_string().contains("was not modified"));
    assert_eq!(
        range_state(&conn, target_id)?,
        ("quarantined".into(), Some(1), None)
    );
    drop(lock_guard);
    drop(conn);

    let error = run_exact_replay(target_id, true, true, "codex")
        .await
        .expect_err("unimplemented exact task should be re-archived");
    assert!(error.to_string().contains("exact replay deferred"));

    let conn = db::open_db()?;
    let (target_status, target_archived, replay_task_id) = range_state(&conn, target_id)?;
    assert_eq!(target_status, "quarantined");
    assert!(target_archived.is_some());
    let replay_task_id = replay_task_id.ok_or_else(|| anyhow::anyhow!("missing replay task"))?;
    let (task_status, task_archived, task_error): (String, Option<i64>, Option<String>) = conn
        .query_row(
            "SELECT status, archived_at_epoch, last_error
             FROM extraction_tasks WHERE id = ?1",
            params![replay_task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    assert_eq!(task_status, "failed");
    assert!(task_archived.is_some());
    assert!(task_error.is_some_and(|value| value.contains("exact replay deferred")));
    assert_eq!(
        range_state(&conn, sibling_id)?,
        ("quarantined".into(), Some(1), None)
    );
    Ok(())
}

#[test]
fn expired_exact_replay_lease_restores_archived_quarantine() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-exact-expired-lease");
    let mut conn = db::open_db()?;
    let range_id = archived_quarantined_range(
        &mut conn,
        "sess-exact-expired",
        db::ExtractionTaskKind::RuleCandidate,
    )?;
    let lease_owner = "worker-v0.5.9-exact-replay-17-23".to_string();
    let task = db::retry_and_claim_extraction_replay_range(
        &mut conn,
        range_id,
        true,
        true,
        &lease_owner,
        60,
    )?;
    conn.execute(
        "UPDATE extraction_tasks SET lease_expires_epoch = ?1 WHERE id = ?2",
        params![chrono::Utc::now().timestamp() - 1, task.id],
    )?;

    assert_eq!(db::release_expired_extraction_task_leases(&conn)?, 1);
    let (range_status, range_archived, replay_task_id) = range_state(&conn, range_id)?;
    assert_eq!(range_status, "quarantined");
    assert!(range_archived.is_some());
    assert_eq!(replay_task_id, Some(task.id));
    let (task_status, task_archived): (String, Option<i64>) = conn.query_row(
        "SELECT status, archived_at_epoch FROM extraction_tasks WHERE id = ?1",
        params![task.id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(task_status, "failed");
    assert!(task_archived.is_some());
    Ok(())
}

#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn exact_replay_drains_same_range_followups_before_reporting_success() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-exact-followup-drain");
    let stub_codex = std::env::temp_dir().join(format!(
        "remem-test-exact-followup-{}-{}.sh",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    super::tests::test_support::install_stub_codex(&stub_codex);
    crate::runtime_config::init_config()?;
    crate::runtime_config::set_config_value(
        "memory_ai.profiles.codex.path",
        stub_codex
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("stub path must be valid utf-8"))?,
    )?;

    let mut conn = db::open_db()?;
    let range_id = archived_quarantined_range(
        &mut conn,
        "sess-exact-followup",
        db::ExtractionTaskKind::ObservationExtract,
    )?;
    drop(conn);

    let result = run_exact_replay(range_id, true, true, "codex").await;
    let _ = std::fs::remove_file(&stub_codex);
    let error = result.expect_err("review-gated graph follow-up must not report exact success");
    assert!(error.to_string().contains("exact replay waiting"));

    let conn = db::open_db()?;
    let (status, archived_at_epoch, replay_task_id) = range_state(&conn, range_id)?;
    assert_eq!(status, "quarantined");
    assert!(archived_at_epoch.is_some());
    let unfinished: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM extraction_tasks
         WHERE replay_range_id = ?1 AND status IN ('pending', 'processing')",
        params![range_id],
        |row| row.get(0),
    )?;
    assert_eq!(
        unfinished, 0,
        "exact follow-ups must not escape to the daemon"
    );
    let failing_kind: String = conn.query_row(
        "SELECT task_kind FROM extraction_tasks WHERE id = ?1",
        params![replay_task_id.ok_or_else(|| anyhow::anyhow!("missing failing task"))?],
        |row| row.get(0),
    )?;
    assert_eq!(
        failing_kind,
        db::ExtractionTaskKind::GraphCandidate.as_str()
    );
    Ok(())
}

#[tokio::test]
async fn exact_replay_full_range_success_reaches_terminal_done() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-exact-success");
    crate::runtime_config::init_config()?;
    let mut conn = db::open_db()?;
    let range_id = archived_quarantined_range(
        &mut conn,
        "sess-exact-success",
        db::ExtractionTaskKind::CapturedGitLink,
    )?;
    drop(conn);

    run_exact_replay(range_id, true, true, "codex").await?;

    let conn = db::open_db()?;
    let (range_status, range_archived, replay_task_id) = range_state(&conn, range_id)?;
    assert_eq!(range_status, "replayed");
    assert!(range_archived.is_none());
    let replay_task_id = replay_task_id.ok_or_else(|| anyhow::anyhow!("missing replay task"))?;
    let task_status: String = conn.query_row(
        "SELECT status FROM extraction_tasks WHERE id = ?1",
        params![replay_task_id],
        |row| row.get(0),
    )?;
    assert_eq!(task_status, "done");
    Ok(())
}
