use anyhow::Result;
use tokio::time::{Duration, Instant};

use super::admission::IntervalAdmission;
use crate::db;

pub(super) const LEGACY_PENDING_MIGRATION_BATCH: i64 = 25;
pub(super) const LEGACY_PENDING_MIGRATION_INTERVAL: Duration = Duration::from_secs(60);

pub(super) fn new_schedule(once: bool, now: Instant) -> IntervalAdmission {
    IntervalAdmission::new(once, now, LEGACY_PENDING_MIGRATION_INTERVAL)
}

pub(super) fn extraction_pipeline_is_idle(conn: &rusqlite::Connection) -> Result<bool> {
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

pub(super) fn should_attempt_legacy_pending_migration(
    conn: &rusqlite::Connection,
    schedule: &mut IntervalAdmission,
    now: Instant,
) -> Result<bool> {
    if db::pending::admin::legacy_pending_auto_bridge_is_exhausted(conn)? {
        return Ok(false);
    }
    if !schedule.is_due(now) {
        return Ok(false);
    }
    if !db::pending::admin::has_auto_actionable_legacy_pending(conn)? {
        db::pending::admin::sync_legacy_pending_bridge_state(conn)?;
        schedule.record_attempt(now);
        return Ok(false);
    }
    extraction_pipeline_is_idle(conn)
}

pub(super) fn record_legacy_pending_migration_outcome(
    schedule: &mut IntervalAdmission,
    now: Instant,
    outcome: &db::pending::admin::AutoLegacyMigrationOutcome,
) {
    if outcome.migrated > 0 || !outcome.yielded_to_current_work {
        schedule.record_attempt(now);
    }
}
