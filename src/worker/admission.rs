use tokio::time::{Duration, Instant};

/// Shared once/daemon interval admission used by cleanup, legacy pending drain,
/// and retrieval enrichment. Queue-specific work stays in those modules; this
/// type only owns the cadence.
#[derive(Debug)]
pub(super) struct IntervalAdmission {
    once: bool,
    attempted_once: bool,
    next_daemon_attempt_at: Instant,
    interval: Duration,
}

impl IntervalAdmission {
    pub(super) fn new(once: bool, now: Instant, interval: Duration) -> Self {
        Self {
            once,
            attempted_once: false,
            next_daemon_attempt_at: now,
            interval,
        }
    }

    pub(super) fn is_due(&self, now: Instant) -> bool {
        if self.once {
            !self.attempted_once
        } else {
            now >= self.next_daemon_attempt_at
        }
    }

    pub(super) fn record_attempt(&mut self, now: Instant) {
        self.attempted_once = true;
        self.next_daemon_attempt_at = now + self.interval;
    }
}
