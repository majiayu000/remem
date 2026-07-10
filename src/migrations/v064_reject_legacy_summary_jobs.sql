-- v064_reject_legacy_summary_jobs: fail closed for in-flight legacy Summary jobs.
--
-- GH684 retires the legacy Summary writer in favor of SessionRollup. Any
-- non-terminal legacy summary job left in the queue at upgrade time would call
-- the retired summary path. Retryable failed Summary jobs would also be
-- reopened by failure lifecycle maintenance. Exhaust both permanently instead
-- of draining or converting a payload whose contract is no longer
-- authoritative.

-- A pre-upgrade daemon may already own a SessionRollup task. Its binary does
-- not run the side effects introduced with this migration, so completing that
-- lease would make the new worker treat the range as fully handled. Requeue
-- claimed rollups before rejecting legacy Summary jobs. Once v064 is applied,
-- the older binary's schema-version gate prevents it from claiming new work;
-- an already-running claim can no longer mark the task done because its lease
-- owner was cleared here.
UPDATE extraction_tasks
SET status = 'pending',
    attempts = 0,
    next_retry_epoch = NULL,
    last_error = NULL,
    failure_class = NULL,
    failed_at_epoch = NULL,
    archived_at_epoch = NULL,
    lease_owner = NULL,
    lease_expires_epoch = NULL,
    updated_at_epoch = CAST(strftime('%s', 'now') AS INTEGER)
WHERE task_kind = 'session_rollup'
  AND status = 'processing';

UPDATE jobs
SET state = 'failed',
    attempt_count = max(COALESCE(attempt_count, 0), COALESCE(max_attempts, attempt_count, 0)),
    next_retry_epoch = 0,
    last_error = 'legacy summary job rejected during GH684 summary retirement upgrade; SessionRollup owns session summary output',
    failure_class = 'permanent',
    failed_at_epoch = COALESCE(failed_at_epoch, CAST(strftime('%s', 'now') AS INTEGER)),
    archived_at_epoch = NULL,
    lease_owner = NULL,
    lease_expires_epoch = NULL,
    updated_at_epoch = CAST(strftime('%s', 'now') AS INTEGER)
WHERE job_type = 'summary'
  AND (
    state NOT IN ('done', 'failed')
    OR (
      state = 'failed'
      AND archived_at_epoch IS NULL
      AND COALESCE(failure_class, 'transient') = 'transient'
      AND COALESCE(attempt_count, 0) < 3
    )
  );
