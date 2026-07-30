use anyhow::Result;
use tokio::time::{sleep, Duration, Instant};

use crate::db;

mod cleanup;
mod job;
mod lock;

// The lease is the maximum time another worker will wait before requeuing a
// job whose owner died, so `JOB_LEASE_SECS` must always exceed
// `JOB_TIMEOUT_SECS`. Otherwise a job that legitimately runs near the
// timeout could be claimed by a second worker before its current owner has
// given up, causing duplicate processing on hard kills. The grace window
// (60s) gives the active worker time to fail the timeout check and release.
const JOB_TIMEOUT_SECS: u64 = 420;
const JOB_LEASE_SECS: i64 = (JOB_TIMEOUT_SECS as i64) + 60;
const _: () = assert!(JOB_LEASE_SECS > JOB_TIMEOUT_SECS as i64);
// Cleanup owns one immediate SQLite transaction and cannot heartbeat its lease
// without contending with itself. Give the bounded maintenance pass a separate
// recovery window instead of routing it through the cancellable job timeout.
const CLEANUP_JOB_LEASE_SECS: i64 = 6 * 60 * 60;
const EXTRACTION_TASK_TIMEOUT_SECS: u64 = JOB_TIMEOUT_SECS;
const EMBEDDING_BACKFILL_IDLE_BATCH_SIZE: i64 = 128;
const RULE_COMPILATION_SWEEP_INTERVAL_SECS: u64 = 60;
const LEGACY_PENDING_MIGRATION_BATCH: i64 = 25;
const LEGACY_PENDING_MIGRATION_INTERVAL: Duration = Duration::from_secs(60);

struct LegacyPendingMigrationSchedule {
    once: bool,
    attempted_once: bool,
    next_daemon_attempt_at: Instant,
}

impl LegacyPendingMigrationSchedule {
    fn new(once: bool, now: Instant) -> Self {
        Self {
            once,
            attempted_once: false,
            next_daemon_attempt_at: now,
        }
    }

    fn is_due(&self, now: Instant) -> bool {
        if self.once {
            !self.attempted_once
        } else {
            now >= self.next_daemon_attempt_at
        }
    }

    fn record_attempt(&mut self, now: Instant) {
        self.attempted_once = true;
        self.next_daemon_attempt_at = now + LEGACY_PENDING_MIGRATION_INTERVAL;
    }
}

fn extraction_pipeline_is_idle(conn: &rusqlite::Connection) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let active: i64 = conn.query_row(
        "SELECT EXISTS(
             SELECT 1
             FROM extraction_tasks
             WHERE (status = 'pending'
                    AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?1))
                OR status = 'processing'
         )",
        [now],
        |row| row.get(0),
    )?;
    Ok(active == 0)
}

fn should_attempt_legacy_pending_migration(
    conn: &rusqlite::Connection,
    schedule: &LegacyPendingMigrationSchedule,
    now: Instant,
) -> Result<bool> {
    if !schedule.is_due(now) {
        return Ok(false);
    }
    extraction_pipeline_is_idle(conn)
}

fn record_legacy_pending_migration_outcome(
    schedule: &mut LegacyPendingMigrationSchedule,
    now: Instant,
    outcome: &db::pending::admin::AutoLegacyMigrationOutcome,
) {
    if outcome.migrated > 0 || !outcome.yielded_to_current_work {
        schedule.record_attempt(now);
    }
}

fn retry_backoff_secs(attempt: i64) -> i64 {
    match attempt {
        0 => 5,
        1 => 15,
        2 => 45,
        3 => 120,
        4 => 300,
        _ => 900,
    }
}

fn record_worker_heartbeat(
    conn: &rusqlite::Connection,
    lease_owner: &str,
    started_at_epoch: i64,
) -> Result<()> {
    db::upsert_worker_heartbeat(
        conn,
        lease_owner,
        i64::from(std::process::id()),
        started_at_epoch,
        chrono::Utc::now().timestamp(),
    )
}

fn run_idle_embedding_backfill(conn: &rusqlite::Connection) -> Result<bool> {
    match crate::retrieval::vector::reindex_memory_embeddings_with_report(
        conn,
        EMBEDDING_BACKFILL_IDLE_BATCH_SIZE,
    ) {
        Ok(report) if report.processed > 0 => {
            crate::log::info(
                "worker",
                &format!(
                    "backfilled {} memory embedding(s) for model={} dimensions={}",
                    report.processed, report.model, report.dimensions
                ),
            );
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(error)
            if crate::retrieval::embedding::is_local_embedding_model_unavailable_error(&error) =>
        {
            crate::log::error(
                "worker",
                &format!("memory embedding backfill deferred: {error}"),
            );
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn recover_expired_jobs(conn: &rusqlite::Connection) -> Result<()> {
    let batch = db::release_expired_job_leases(conn)?;
    for outcome in batch.outcomes {
        match outcome {
            db::ExpiredJobLeaseOutcome::Requeued {
                source_id,
                identity_kind,
            } => crate::log::warn(
                "worker",
                &format!(
                    "expired job recovery requeued source_id={source_id} identity={}",
                    identity_kind.as_str()
                ),
            ),
            db::ExpiredJobLeaseOutcome::Coalesced {
                source_id,
                canonical_id,
                identity_kind,
            } => crate::log::warn(
                "worker",
                &format!(
                    "expired job recovery coalesced source_id={source_id} canonical_id={canonical_id} identity={}",
                    identity_kind.as_str()
                ),
            ),
        }
    }
    Ok(())
}

fn mark_successful_job(
    conn: &rusqlite::Connection,
    job_id: i64,
    job_type: db::JobType,
    project: &str,
    lease_owner: &str,
) -> Result<()> {
    if let Err(error) = db::mark_job_done(conn, job_id, lease_owner) {
        crate::log::error(
            "worker",
            &format!(
                "job transition failed id={job_id} operation=done job_type={} project_hash={:016x} expected_owner={lease_owner} error={error}",
                job_type.as_str(),
                db::deterministic_hash(project.as_bytes())
            ),
        );
        return Err(error);
    }
    crate::log::info("worker", &format!("done id={job_id}"));
    Ok(())
}

fn record_failed_job_transition(
    conn: &rusqlite::Connection,
    job_id: i64,
    job_type: db::JobType,
    project: &str,
    lease_owner: &str,
    error_message: &str,
    backoff_secs: i64,
) -> Result<()> {
    let transition = match db::mark_job_failed_or_retry(
        conn,
        job_id,
        lease_owner,
        error_message,
        backoff_secs,
    ) {
        Ok(transition) => transition,
        Err(error) => {
            crate::log::error(
                "worker",
                &format!(
                    "job transition failed id={job_id} operation=retry job_type={} project_hash={:016x} expected_owner={lease_owner} error={error}",
                    job_type.as_str(),
                    db::deterministic_hash(project.as_bytes())
                ),
            );
            return Err(error);
        }
    };
    match transition {
        db::JobTransitionOutcome::Transitioned => crate::log::warn(
            "worker",
            &format!(
                "job id={job_id} failed: {} (retry in {backoff_secs}s)",
                crate::db::truncate_str(error_message, 300)
            ),
        ),
        db::JobTransitionOutcome::Coalesced {
            source_id,
            canonical_id,
            identity_kind,
        } => crate::log::info(
            "worker",
            &format!(
                "job retry coalesced source_id={source_id} canonical_id={canonical_id} identity={}",
                identity_kind.as_str()
            ),
        ),
    }
    Ok(())
}

pub async fn run_exact_replay(
    range_id: i64,
    acknowledge_quarantine: bool,
    include_archived: bool,
    profile: &str,
) -> Result<()> {
    let started_at_epoch = chrono::Utc::now().timestamp();
    let resolved = crate::runtime_config::resolve_memory_ai_profile(
        crate::runtime_config::MemoryAiSelection {
            host: None,
            profile: Some(profile),
        },
    )?;
    let lease_owner =
        db::exact_replay_worker_owner(std::process::id(), chrono::Utc::now().timestamp_millis());
    let Some(_singleton) = lock::acquire_worker_singleton()? else {
        anyhow::bail!("worker singleton is held; exact replay range {range_id} was not modified");
    };

    crate::log::info(
        "worker",
        &format!(
            "exact replay start range_id={range_id} profile={} executor={} model={}",
            resolved.profile_name,
            resolved.executor.as_str(),
            resolved.model.as_deref().unwrap_or("<default>")
        ),
    );
    let mut conn = db::open_db()?;
    record_worker_heartbeat(&conn, &lease_owner, started_at_epoch)?;
    let task = db::retry_and_claim_extraction_replay_range(
        &mut conn,
        range_id,
        acknowledge_quarantine,
        include_archived,
        &lease_owner,
        JOB_LEASE_SECS,
    )?;
    drop(conn);

    crate::extraction_worker::run_claimed_exact(
        task,
        &resolved,
        &lease_owner,
        EXTRACTION_TASK_TIMEOUT_SECS,
    )
    .await?;
    crate::log::info(
        "worker",
        &format!(
            "exact replay done range_id={range_id} profile={} executor={}",
            resolved.profile_name,
            resolved.executor.as_str()
        ),
    );
    Ok(())
}

pub async fn run(once: bool, idle_sleep_ms: u64) -> Result<()> {
    let started_at_epoch = chrono::Utc::now().timestamp();
    let mode = if once { "once" } else { "daemon" };
    let lease_owner = db::current_worker_owner(
        mode,
        std::process::id(),
        chrono::Utc::now().timestamp_millis(),
    );
    let Some(_singleton) = lock::acquire_worker_singleton_for_mode(once)? else {
        crate::log::info("worker", "worker already running, exiting");
        return Ok(());
    };
    crate::log::info(
        "worker",
        &format!("start owner={} mode={}", lease_owner, mode),
    );
    {
        let conn = db::open_db()?;
        record_worker_heartbeat(&conn, &lease_owner, started_at_epoch)?;
    }

    let mut legacy_pending_migration_schedule =
        LegacyPendingMigrationSchedule::new(once, Instant::now());
    let mut cleanup_probe_schedule = cleanup::CleanupProbeSchedule::new(once, Instant::now());
    let mut next_rule_compilation_sweep_at = Instant::now();
    loop {
        if Instant::now() >= next_rule_compilation_sweep_at {
            match job::run_rule_compilation_sweep().await {
                Ok(outcome) => {
                    if outcome.failures > 0 {
                        crate::log::error(
                            "rules",
                            &format!(
                                "rule compilation sweep completed with {}/{} project failure(s)",
                                outcome.failures, outcome.projects_seen
                            ),
                        );
                    }
                    if outcome.artifacts_changed > 0 {
                        crate::log::info(
                            "rules",
                            &format!(
                                "rule compilation sweep rebuilt {}/{} project artifact(s)",
                                outcome.artifacts_changed, outcome.projects_seen
                            ),
                        );
                    }
                }
                Err(error) => crate::log::error(
                    "rules",
                    &format!("rule compilation sweep skipped after setup failure: {error}"),
                ),
            }
            next_rule_compilation_sweep_at =
                Instant::now() + Duration::from_secs(RULE_COMPILATION_SWEEP_INTERVAL_SECS);
        }
        let mut conn = db::open_db()?;
        record_worker_heartbeat(&conn, &lease_owner, started_at_epoch)?;
        recover_expired_jobs(&conn)?;
        let recovered_extraction = db::release_expired_extraction_task_leases(&conn)?;
        if recovered_extraction > 0 {
            crate::log::warn(
                "worker",
                &format!(
                    "released {} expired extraction task lease(s)",
                    recovered_extraction
                ),
            );
        }
        db::maintain_failure_lifecycle(&conn)?;
        let cleanup_probe_now = Instant::now();
        if let Some(decision) =
            cleanup::enqueue_if_due(&conn, &mut cleanup_probe_schedule, cleanup_probe_now)?
        {
            crate::log::info(
                "worker",
                &format!("automatic cleanup schedule decision={decision:?}"),
            );
        }
        if let Some(cleanup_job) =
            db::claim_ready_cleanup_job(&mut conn, &lease_owner, CLEANUP_JOB_LEASE_SECS)?
        {
            crate::log::info(
                "worker",
                &format!(
                    "claimed id={} type=cleanup attempt={}/{}",
                    cleanup_job.id,
                    cleanup_job.attempt_count + 1,
                    cleanup_job.max_attempts
                ),
            );
            drop(conn);
            match cleanup::execute_claimed(&cleanup_job, &lease_owner).await {
                Ok(execution) => crate::log::info(
                    "worker",
                    &format!(
                        "automatic cleanup done id={} applied={:?}",
                        cleanup_job.id, execution.applied
                    ),
                ),
                Err(error) => {
                    let message = cleanup::safe_failure_message(&error);
                    let backoff = retry_backoff_secs(cleanup_job.attempt_count);
                    let conn = db::open_db()?;
                    record_failed_job_transition(
                        &conn,
                        cleanup_job.id,
                        cleanup_job.job_type,
                        &cleanup_job.project,
                        &lease_owner,
                        &message,
                        backoff,
                    )?;
                }
            }
            continue;
        }
        let migration_now = Instant::now();
        if should_attempt_legacy_pending_migration(
            &conn,
            &legacy_pending_migration_schedule,
            migration_now,
        )? {
            match db::pending::admin::auto_migrate_actionable_legacy_pending(
                &mut conn,
                LEGACY_PENDING_MIGRATION_BATCH,
            ) {
                Ok(outcome) => {
                    record_legacy_pending_migration_outcome(
                        &mut legacy_pending_migration_schedule,
                        Instant::now(),
                        &outcome,
                    );
                    if outcome.migrated > 0 {
                        crate::log::info(
                            "worker",
                            &format!(
                                "surface=pending_observation outcome=migrated count={}",
                                outcome.migrated
                            ),
                        );
                    }
                }
                Err(error) => {
                    legacy_pending_migration_schedule.record_attempt(Instant::now());
                    crate::log::error(
                        "worker",
                        &format!("legacy pending auto-migration failed: {error}"),
                    );
                }
            }
        }
        if crate::extraction_worker::run_next(
            &lease_owner,
            JOB_LEASE_SECS,
            EXTRACTION_TASK_TIMEOUT_SECS,
        )
        .await?
        {
            continue;
        }

        if let Some(job) = db::claim_next_job(&mut conn, &lease_owner, JOB_LEASE_SECS)? {
            crate::log::info(
                "worker",
                &format!(
                    "claimed id={} type={} project={} attempt={}/{}",
                    job.id,
                    job.job_type.as_str(),
                    job.project,
                    job.attempt_count + 1,
                    job.max_attempts
                ),
            );

            let timed = tokio::time::timeout(
                Duration::from_secs(JOB_TIMEOUT_SECS),
                job::process_job(&job),
            )
            .await;
            let conn = db::open_db()?;
            match timed {
                Ok(Ok(())) => {
                    mark_successful_job(&conn, job.id, job.job_type, &job.project, &lease_owner)?;
                }
                Ok(Err(e)) => {
                    let msg = e.to_string();
                    let backoff = retry_backoff_secs(job.attempt_count);
                    record_failed_job_transition(
                        &conn,
                        job.id,
                        job.job_type,
                        &job.project,
                        &lease_owner,
                        &msg,
                        backoff,
                    )?;
                }
                Err(_) => {
                    let msg = format!("job timed out after {}s", JOB_TIMEOUT_SECS);
                    let backoff = retry_backoff_secs(job.attempt_count);
                    record_failed_job_transition(
                        &conn,
                        job.id,
                        job.job_type,
                        &job.project,
                        &lease_owner,
                        &msg,
                        backoff,
                    )?;
                }
            }
            continue;
        }

        if crate::memory::retrieval_enrichment::run_idle_retrieval_enrichment(&lease_owner).await? {
            continue;
        }

        if run_idle_embedding_backfill(&conn)? {
            continue;
        }

        if once {
            break;
        }
        sleep(Duration::from_millis(idle_sleep_ms.max(100))).await;
        continue;
    }

    if !once {
        let conn = db::open_db()?;
        record_worker_heartbeat(&conn, &lease_owner, started_at_epoch)?;
    }
    crate::log::info("worker", "stopped");
    Ok(())
}

#[cfg(test)]
mod cleanup_tests;
#[cfg(test)]
mod exact_tests;
#[cfg(test)]
mod legacy_pending_schedule_tests {
    use super::*;
    use crate::db::{test_support::ScopedTestDataDir, CaptureEventInput, ExtractionTaskKind};
    use rusqlite::params;

    fn setup_conn() -> anyhow::Result<rusqlite::Connection> {
        let conn = rusqlite::Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    fn seed_extraction_task(conn: &rusqlite::Connection) -> anyhow::Result<i64> {
        let outcome = crate::db::record_captured_event(
            conn,
            &CaptureEventInput {
                host: "codex-cli",
                session_id: "legacy-bridge-backpressure",
                project: "/tmp/remem-legacy-bridge",
                cwd: None,
                event_type: "tool_result",
                role: None,
                tool_name: Some("Bash"),
                content: r#"{"summary":"existing extraction work"}"#,
                task_kind: Some(ExtractionTaskKind::ObservationExtract),
            },
        )?;
        outcome
            .extraction_task_id
            .ok_or_else(|| anyhow::anyhow!("capture did not enqueue extraction"))
    }

    #[test]
    fn once_schedule_allows_only_one_migration_attempt() {
        let started_at = Instant::now();
        let mut schedule = LegacyPendingMigrationSchedule::new(true, started_at);

        assert!(schedule.is_due(started_at));
        schedule.record_attempt(started_at);
        assert!(!schedule.is_due(started_at));
        assert!(!schedule
            .is_due(started_at + LEGACY_PENDING_MIGRATION_INTERVAL + Duration::from_secs(1)));
    }

    #[test]
    fn daemon_schedule_waits_sixty_seconds_between_attempts() {
        let started_at = Instant::now();
        let mut schedule = LegacyPendingMigrationSchedule::new(false, started_at);

        assert!(schedule.is_due(started_at));
        schedule.record_attempt(started_at);
        assert!(!schedule
            .is_due(started_at + LEGACY_PENDING_MIGRATION_INTERVAL - Duration::from_millis(1)));
        assert!(schedule.is_due(started_at + LEGACY_PENDING_MIGRATION_INTERVAL));
    }

    #[test]
    fn due_pending_and_processing_apply_backpressure_but_delayed_does_not() -> anyhow::Result<()> {
        let conn = setup_conn()?;
        let task_id = seed_extraction_task(&conn)?;
        let now = chrono::Utc::now().timestamp();

        assert!(!extraction_pipeline_is_idle(&conn)?);
        conn.execute(
            "UPDATE extraction_tasks
             SET status = 'pending', next_retry_epoch = ?1
             WHERE id = ?2",
            params![now + 3_600, task_id],
        )?;
        assert!(extraction_pipeline_is_idle(&conn)?);

        conn.execute(
            "UPDATE extraction_tasks
             SET status = 'processing',
                 lease_owner = 'existing-worker',
                 lease_expires_epoch = ?1
             WHERE id = ?2",
            params![now + 300, task_id],
        )?;
        assert!(!extraction_pipeline_is_idle(&conn)?);

        conn.execute(
            "UPDATE extraction_tasks
             SET status = 'done',
                 lease_owner = NULL,
                 lease_expires_epoch = NULL
             WHERE id = ?1",
            [task_id],
        )?;
        assert!(extraction_pipeline_is_idle(&conn)?);
        Ok(())
    }

    #[test]
    fn active_extraction_work_does_not_consume_once_migration_slot() -> anyhow::Result<()> {
        let conn = setup_conn()?;
        let task_id = seed_extraction_task(&conn)?;
        let started_at = Instant::now();
        let mut schedule = LegacyPendingMigrationSchedule::new(true, started_at);

        assert!(!should_attempt_legacy_pending_migration(
            &conn, &schedule, started_at
        )?);
        assert!(schedule.is_due(started_at));

        conn.execute(
            "UPDATE extraction_tasks SET status = 'done' WHERE id = ?1",
            [task_id],
        )?;
        assert!(should_attempt_legacy_pending_migration(
            &conn, &schedule, started_at
        )?);
        schedule.record_attempt(started_at);
        assert!(!should_attempt_legacy_pending_migration(
            &conn, &schedule, started_at
        )?);
        Ok(())
    }

    #[test]
    fn zero_progress_yield_does_not_consume_once_migration_slot() {
        let started_at = Instant::now();
        let mut schedule = LegacyPendingMigrationSchedule::new(true, started_at);
        let outcome = db::pending::admin::AutoLegacyMigrationOutcome {
            migrated: 0,
            yielded_to_current_work: true,
        };

        record_legacy_pending_migration_outcome(&mut schedule, started_at, &outcome);

        assert!(schedule.is_due(started_at));
    }

    #[test]
    fn partial_progress_yield_consumes_once_migration_slot() {
        let started_at = Instant::now();
        let mut schedule = LegacyPendingMigrationSchedule::new(true, started_at);
        let outcome = db::pending::admin::AutoLegacyMigrationOutcome {
            migrated: 1,
            yielded_to_current_work: true,
        };

        record_legacy_pending_migration_outcome(&mut schedule, started_at, &outcome);

        assert!(!schedule.is_due(started_at));
    }

    #[test]
    fn completed_zero_progress_attempt_consumes_once_migration_slot() {
        let started_at = Instant::now();
        let mut schedule = LegacyPendingMigrationSchedule::new(true, started_at);
        let outcome = db::pending::admin::AutoLegacyMigrationOutcome::default();

        record_legacy_pending_migration_outcome(&mut schedule, started_at, &outcome);

        assert!(!schedule.is_due(started_at));
    }

    #[test]
    fn lifecycle_requeue_of_due_extraction_work_blocks_legacy_migration() -> anyhow::Result<()> {
        let conn = setup_conn()?;
        let task_id = seed_extraction_task(&conn)?;
        let now_epoch = chrono::Utc::now().timestamp();
        conn.execute(
            "UPDATE extraction_tasks
             SET status = 'failed',
                 attempts = 0,
                 next_retry_epoch = 0,
                 failure_class = 'transient',
                 failed_at_epoch = ?1,
                 updated_at_epoch = ?1
             WHERE id = ?2",
            params![now_epoch - 1_000, task_id],
        )?;
        assert!(extraction_pipeline_is_idle(&conn)?);

        let maintenance = crate::db::maintain_failure_lifecycle(&conn)?;

        assert_eq!(maintenance.retried_extraction_tasks, 1);
        let schedule = LegacyPendingMigrationSchedule::new(true, Instant::now());
        assert!(!should_attempt_legacy_pending_migration(
            &conn,
            &schedule,
            Instant::now()
        )?);
        Ok(())
    }

    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn worker_once_restart_redrives_failed_legacy_backlog() -> anyhow::Result<()> {
        let data_dir = ScopedTestDataDir::new("worker-legacy-restart");
        std::fs::create_dir_all(&data_dir.path)?;
        let stub_codex = data_dir.path.join("codex-stub.sh");
        super::tests::test_support::install_stub_codex(&stub_codex);
        crate::runtime_config::init_config()?;
        crate::runtime_config::set_config_value(
            "memory_ai.profiles.codex.path",
            stub_codex
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("stub path must be valid utf-8"))?,
        )?;
        let conn = crate::db::open_db()?;
        let pending_id = crate::db::test_support::insert_legacy_pending_fixture(
            &conn,
            crate::runtime_config::CODEX_HOST,
            "worker-died-before-retry",
            "/tmp/remem-legacy-restart",
            "Bash",
            Some(r#"{"cmd":"printf important"}"#),
            Some(r#"{"output":"important"}"#),
            Some("/tmp/remem-legacy-restart"),
        )?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "UPDATE pending_observations
             SET status = 'failed', failure_class = 'transient',
                 attempt_count = 3, next_retry_epoch = ?2,
                 failed_at_epoch = ?3, archived_at_epoch = ?4,
                 last_error = 'worker terminated during processing'
             WHERE id = ?1",
            params![pending_id, now - 1, now - 500, now - 400],
        )?;
        drop(conn);

        run(true, 10).await?;

        let conn = crate::db::open_db()?;
        let source: (String, i64, Option<i64>) = conn.query_row(
            "SELECT status, attempt_count, archived_at_epoch
             FROM pending_observations WHERE id = ?1",
            [pending_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(source, ("migrated".to_string(), 0, None));
        let failed_backlog: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pending_observations WHERE status = 'failed'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(failed_backlog, 0);
        let captured: i64 = conn.query_row(
            "SELECT COUNT(*) FROM captured_events
             WHERE event_id = ?1",
            [format!("legacy-pending-{pending_id}")],
            |row| row.get(0),
        )?;
        assert_eq!(captured, 1);
        let observations: i64 =
            conn.query_row("SELECT COUNT(*) FROM observations", [], |row| row.get(0))?;
        assert!(observations >= 1);
        Ok(())
    }
}
#[cfg(all(test, unix))]
mod tests;
