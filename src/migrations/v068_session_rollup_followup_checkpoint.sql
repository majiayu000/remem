-- v068_session_rollup_followup_checkpoint: record one durable Compress/Dream
-- scheduling decision for each persisted SessionRollup event range. Exact
-- ranges created before this migration are explicitly marked legacy_unknown:
-- their prior job decisions cannot be reconstructed safely, so retries must
-- preserve the ambiguity instead of scheduling replacements.

ALTER TABLE session_summaries
    ADD COLUMN followup_scheduling_completed_at_epoch INTEGER;

ALTER TABLE session_summaries
    ADD COLUMN followup_scheduling_state TEXT
    DEFAULT 'legacy_unknown'
    CHECK (followup_scheduling_state IS NULL OR followup_scheduling_state IN (
        'claimed', 'completed', 'legacy_unknown'
    ));

ALTER TABLE session_summaries
    ADD COLUMN followup_compress_job_id INTEGER;

ALTER TABLE session_summaries
    ADD COLUMN followup_dream_disposition TEXT
    DEFAULT 'legacy_unknown'
    CHECK (followup_dream_disposition IS NULL OR followup_dream_disposition IN (
        'enqueued', 'coalesced_inflight', 'suppressed_recent_done', 'legacy_unknown'
    ));

ALTER TABLE session_summaries
    ADD COLUMN followup_dream_job_id INTEGER;

UPDATE session_summaries
SET followup_scheduling_state = 'legacy_unknown',
    followup_dream_disposition = 'legacy_unknown'
WHERE session_row_id IS NOT NULL
  AND covered_from_event_id IS NOT NULL
  AND covered_to_event_id IS NOT NULL;

UPDATE session_summaries
SET followup_scheduling_state = NULL,
    followup_dream_disposition = NULL
WHERE session_row_id IS NULL
   OR covered_from_event_id IS NULL
   OR covered_to_event_id IS NULL;

-- A pre-v068 worker may already be awaiting AI when this migration runs. Its
-- late exact-range INSERT omits the new columns and therefore receives the
-- legacy_unknown defaults above. Clear its task lease as an additional fence:
-- the old worker cannot mark the task done, and the current worker will retry
-- without inferring replacement jobs from terminal history.
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
