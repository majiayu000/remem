-- v055_failure_lifecycle: classify, age, archive, and audit terminal
-- queue failures without deleting their source rows automatically.

ALTER TABLE pending_observations ADD COLUMN failure_class TEXT;
ALTER TABLE pending_observations ADD COLUMN failed_at_epoch INTEGER;
ALTER TABLE pending_observations ADD COLUMN archived_at_epoch INTEGER;

ALTER TABLE extraction_tasks ADD COLUMN failure_class TEXT;
ALTER TABLE extraction_tasks ADD COLUMN failed_at_epoch INTEGER;
ALTER TABLE extraction_tasks ADD COLUMN archived_at_epoch INTEGER;

ALTER TABLE extraction_replay_ranges ADD COLUMN failure_class TEXT;
ALTER TABLE extraction_replay_ranges ADD COLUMN failed_at_epoch INTEGER;
ALTER TABLE extraction_replay_ranges ADD COLUMN archived_at_epoch INTEGER;

ALTER TABLE jobs ADD COLUMN failure_class TEXT;
ALTER TABLE jobs ADD COLUMN failed_at_epoch INTEGER;
ALTER TABLE jobs ADD COLUMN archived_at_epoch INTEGER;

CREATE TABLE IF NOT EXISTS failure_lifecycle_daily (
    day_epoch INTEGER NOT NULL,
    surface TEXT NOT NULL,
    failure_class TEXT NOT NULL,
    archived_count INTEGER NOT NULL DEFAULT 0,
    purged_count INTEGER NOT NULL DEFAULT 0,
    oldest_failed_at_epoch INTEGER,
    newest_failed_at_epoch INTEGER,
    last_rollup_epoch INTEGER NOT NULL,
    PRIMARY KEY(day_epoch, surface, failure_class)
);

CREATE INDEX IF NOT EXISTS idx_failure_lifecycle_daily_surface
    ON failure_lifecycle_daily(surface, day_epoch);

CREATE INDEX IF NOT EXISTS idx_pending_observations_failure_lifecycle
    ON pending_observations(status, archived_at_epoch, failed_at_epoch, failure_class);

CREATE INDEX IF NOT EXISTS idx_extraction_tasks_failure_lifecycle
    ON extraction_tasks(status, archived_at_epoch, failed_at_epoch, failure_class);

CREATE INDEX IF NOT EXISTS idx_extraction_replay_ranges_failure_lifecycle
    ON extraction_replay_ranges(status, archived_at_epoch, failed_at_epoch, failure_class);

CREATE INDEX IF NOT EXISTS idx_jobs_failure_lifecycle
    ON jobs(state, archived_at_epoch, failed_at_epoch, failure_class);

UPDATE pending_observations
SET failure_class = CASE
        WHEN lower(COALESCE(last_error, '')) LIKE '%schema%'
          OR lower(COALESCE(last_error, '')) LIKE '%vocab%'
          OR lower(COALESCE(last_error, '')) LIKE '%malformed%'
          OR lower(COALESCE(last_error, '')) LIKE '%invalid payload%'
          OR lower(COALESCE(last_error, '')) LIKE '%invalid json%'
          OR lower(COALESCE(last_error, '')) LIKE '%unsupported version%'
          OR lower(COALESCE(last_error, '')) LIKE '%missing evidence%'
          OR lower(COALESCE(last_error, '')) LIKE '%not implemented%'
        THEN 'permanent'
        ELSE 'transient'
    END,
    failed_at_epoch = COALESCE(NULLIF(updated_at_epoch, 0), created_at_epoch),
    attempt_count = max(COALESCE(attempt_count, 0), 3)
WHERE status = 'failed';

UPDATE extraction_tasks
SET failure_class = CASE
        WHEN lower(COALESCE(last_error, '')) LIKE '%schema%'
          OR lower(COALESCE(last_error, '')) LIKE '%vocab%'
          OR lower(COALESCE(last_error, '')) LIKE '%malformed%'
          OR lower(COALESCE(last_error, '')) LIKE '%invalid payload%'
          OR lower(COALESCE(last_error, '')) LIKE '%invalid json%'
          OR lower(COALESCE(last_error, '')) LIKE '%unsupported version%'
          OR lower(COALESCE(last_error, '')) LIKE '%missing evidence%'
          OR lower(COALESCE(last_error, '')) LIKE '%not implemented%'
        THEN 'permanent'
        ELSE 'transient'
    END,
    failed_at_epoch = COALESCE(NULLIF(updated_at_epoch, 0), created_at_epoch),
    attempts = max(COALESCE(attempts, 0), 3)
WHERE status = 'failed';

UPDATE extraction_replay_ranges
SET failure_class = CASE
        WHEN lower(COALESCE(last_error, '')) LIKE '%schema%'
          OR lower(COALESCE(last_error, '')) LIKE '%vocab%'
          OR lower(COALESCE(last_error, '')) LIKE '%malformed%'
          OR lower(COALESCE(last_error, '')) LIKE '%invalid payload%'
          OR lower(COALESCE(last_error, '')) LIKE '%invalid json%'
          OR lower(COALESCE(last_error, '')) LIKE '%unsupported version%'
          OR lower(COALESCE(last_error, '')) LIKE '%missing evidence%'
          OR lower(COALESCE(last_error, '')) LIKE '%not implemented%'
        THEN 'permanent'
        ELSE 'transient'
    END,
    failed_at_epoch = COALESCE(NULLIF(updated_at_epoch, 0), created_at_epoch),
    attempts = max(COALESCE(attempts, 0), 3)
WHERE status IN ('pending', 'failed', 'quarantined');

UPDATE jobs
SET failure_class = CASE
        WHEN lower(COALESCE(last_error, '')) LIKE '%schema%'
          OR lower(COALESCE(last_error, '')) LIKE '%vocab%'
          OR lower(COALESCE(last_error, '')) LIKE '%malformed%'
          OR lower(COALESCE(last_error, '')) LIKE '%invalid payload%'
          OR lower(COALESCE(last_error, '')) LIKE '%invalid json%'
          OR lower(COALESCE(last_error, '')) LIKE '%unsupported version%'
          OR lower(COALESCE(last_error, '')) LIKE '%missing evidence%'
          OR lower(COALESCE(last_error, '')) LIKE '%not implemented%'
        THEN 'permanent'
        ELSE 'transient'
    END,
    failed_at_epoch = COALESCE(NULLIF(updated_at_epoch, 0), created_at_epoch),
    attempt_count = max(COALESCE(attempt_count, 0), 3)
WHERE state = 'failed';
