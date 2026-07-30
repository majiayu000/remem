use anyhow::{Context, Result};
use tokio::time::{Duration, Instant};

use crate::{db, maintenance};

const CLEANUP_PROBE_INTERVAL: Duration = Duration::from_secs(60);

pub(super) struct CleanupProbeSchedule {
    once: bool,
    attempted_once: bool,
    next_daemon_probe_at: Instant,
}

impl CleanupProbeSchedule {
    pub(super) fn new(once: bool, now: Instant) -> Self {
        Self {
            once,
            attempted_once: false,
            next_daemon_probe_at: now,
        }
    }

    pub(super) fn is_due(&self, now: Instant) -> bool {
        if self.once {
            !self.attempted_once
        } else {
            now >= self.next_daemon_probe_at
        }
    }

    pub(super) fn record_probe(&mut self, now: Instant) {
        self.attempted_once = true;
        self.next_daemon_probe_at = now + CLEANUP_PROBE_INTERVAL;
    }
}

pub(super) fn enqueue_if_due(
    conn: &rusqlite::Connection,
    schedule: &mut CleanupProbeSchedule,
    now: Instant,
) -> Result<Option<db::CleanupEnqueueDecision>> {
    if !schedule.is_due(now) {
        return Ok(None);
    }
    schedule.record_probe(now);
    db::maybe_enqueue_cleanup_job(conn)
        .map(Some)
        .context("schedule automatic lifecycle cleanup")
}

pub(super) async fn execute_claimed(
    job: &db::Job,
    lease_owner: &str,
) -> Result<maintenance::CleanupExecution> {
    anyhow::ensure!(
        job.job_type == db::JobType::Cleanup,
        "dedicated cleanup executor received {} job",
        job.job_type.as_str()
    );
    let job_id = job.id;
    let lease_owner = lease_owner.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db::open_db().context("open database for automatic lifecycle cleanup")?;
        maintenance::execute_automatic_cleanup_job(
            &conn,
            job_id,
            &lease_owner,
            chrono::Utc::now().timestamp(),
        )
    })
    .await
    .context("join automatic lifecycle cleanup task")?
}

pub(super) fn safe_failure_message(error: &anyhow::Error) -> String {
    maintenance::safe_cleanup_error(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn once_schedule_probes_only_once() {
        let started = Instant::now();
        let mut schedule = CleanupProbeSchedule::new(true, started);

        assert!(schedule.is_due(started));
        schedule.record_probe(started);
        assert!(!schedule.is_due(started + CLEANUP_PROBE_INTERVAL));
    }

    #[test]
    fn daemon_schedule_probes_once_per_minute() {
        let started = Instant::now();
        let mut schedule = CleanupProbeSchedule::new(false, started);

        assert!(schedule.is_due(started));
        schedule.record_probe(started);
        assert!(!schedule.is_due(started + CLEANUP_PROBE_INTERVAL - Duration::from_millis(1)));
        assert!(schedule.is_due(started + CLEANUP_PROBE_INTERVAL));
    }

    #[test]
    fn cleanup_failure_message_is_bounded_and_redacted() {
        let error = anyhow::anyhow!("api_key=super-secret-{}", "x".repeat(2_000));

        let message = safe_failure_message(&error);

        assert!(message.contains("[REDACTED]"));
        assert!(!message.contains("super-secret"));
        assert!(message.len() <= 1_000);
    }
}
