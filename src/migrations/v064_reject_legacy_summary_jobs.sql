-- v064_reject_legacy_summary_jobs: fail closed for in-flight legacy Summary jobs.
--
-- GH684 retires the legacy Summary writer in favor of SessionRollup. Any
-- non-terminal legacy summary job left in the queue at upgrade time would call
-- the retired summary path, so exhaust it permanently instead of draining or
-- converting a payload whose contract is no longer authoritative.

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
  AND state NOT IN ('done', 'failed');
