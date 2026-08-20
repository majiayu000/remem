use anyhow::Result;
use tokio::time::{sleep, Duration, Instant};

use crate::db;

mod admission;
mod cleanup;
mod job;
mod legacy_pending;
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
const RETRIEVAL_ENRICHMENT_INTERVAL: Duration = Duration::from_secs(60);
const ONCE_WORKER_MAX_WORK_ITEMS: usize = 4;
const ONCE_WORKER_MAX_ELAPSED: Duration = Duration::from_secs(180);

struct WorkerRunBudget {
    once: bool,
    started_at: Instant,
    work_items: usize,
}

impl WorkerRunBudget {
    fn new(once: bool, started_at: Instant) -> Self {
        Self {
            once,
            started_at,
            work_items: 0,
        }
    }

    fn remaining_work_items(&self, now: Instant) -> usize {
        if !self.once {
            return usize::MAX;
        }
        if now.duration_since(self.started_at) >= ONCE_WORKER_MAX_ELAPSED {
            return 0;
        }
        ONCE_WORKER_MAX_WORK_ITEMS.saturating_sub(self.work_items)
    }

    fn record_work_items(&mut self, count: usize) {
        self.work_items = self.work_items.saturating_add(count);
    }

    fn exhaustion_reason(&self, now: Instant) -> Option<&'static str> {
        if !self.once {
            None
        } else if self.work_items >= ONCE_WORKER_MAX_WORK_ITEMS {
            Some("work_item_limit")
        } else if now.duration_since(self.started_at) >= ONCE_WORKER_MAX_ELAPSED {
            Some("elapsed_limit")
        } else {
            None
        }
    }
}

fn stop_for_exhausted_once_budget(run_budget: &WorkerRunBudget, now: Instant) -> bool {
    let Some(reason) = run_budget.exhaustion_reason(now) else {
        return false;
    };
    crate::log::info(
        "worker",
        &format!(
            "once budget exhausted reason={reason} work_items={} max_work_items={} max_elapsed_secs={}",
            run_budget.work_items,
            ONCE_WORKER_MAX_WORK_ITEMS,
            ONCE_WORKER_MAX_ELAPSED.as_secs(),
        ),
    );
    true
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

    let mut legacy_pending_migration_schedule = legacy_pending::new_schedule(once, Instant::now());
    let mut retrieval_enrichment_schedule =
        admission::IntervalAdmission::new(once, Instant::now(), RETRIEVAL_ENRICHMENT_INTERVAL);
    let mut run_budget = WorkerRunBudget::new(once, Instant::now());
    let mut cleanup_probe_schedule =
        admission::IntervalAdmission::new(once, Instant::now(), cleanup::CLEANUP_PROBE_INTERVAL);
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
        if legacy_pending::should_attempt_legacy_pending_migration(
            &conn,
            &mut legacy_pending_migration_schedule,
            migration_now,
        )? {
            match db::pending::admin::auto_migrate_actionable_legacy_pending(
                &mut conn,
                legacy_pending::LEGACY_PENDING_MIGRATION_BATCH,
            ) {
                Ok(outcome) => {
                    legacy_pending::record_legacy_pending_migration_outcome(
                        &mut legacy_pending_migration_schedule,
                        Instant::now(),
                        &outcome,
                    );
                    if !outcome.yielded_to_current_work {
                        db::pending::admin::sync_legacy_pending_bridge_state(&conn)?;
                    }
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
        // Local maintenance above may itself take time. Recheck immediately
        // before every queue that can enter a provider-backed task so the
        // 180-second rule is an admission deadline, not merely a loop hint.
        if stop_for_exhausted_once_budget(&run_budget, Instant::now()) {
            break;
        }
        if crate::extraction_worker::run_next(
            &lease_owner,
            JOB_LEASE_SECS,
            EXTRACTION_TASK_TIMEOUT_SECS,
        )
        .await?
        {
            run_budget.record_work_items(1);
            continue;
        }

        if stop_for_exhausted_once_budget(&run_budget, Instant::now()) {
            break;
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
            if matches!(job.job_type, db::JobType::Compress | db::JobType::Dream) {
                run_budget.record_work_items(1);
            }
            continue;
        }

        let enrichment_now = Instant::now();
        if retrieval_enrichment_schedule.is_due(enrichment_now) {
            let outcome = crate::memory::retrieval_enrichment::run_idle_retrieval_enrichment(
                &lease_owner,
                run_budget.remaining_work_items(enrichment_now),
            )
            .await?;
            retrieval_enrichment_schedule.record_attempt(Instant::now());
            run_budget.record_work_items(outcome.attempted);
            if outcome.attempted > 0 {
                continue;
            }
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
mod legacy_pending_schedule_tests;
#[cfg(test)]
mod retrieval_enrichment_schedule_tests;
#[cfg(test)]
mod run_budget_tests;
#[cfg(all(test, unix))]
mod tests;
