-- v068_session_rollup_followup_checkpoint: record one durable Compress/Dream
-- scheduling decision for each persisted SessionRollup event range. Exact
-- ranges created before this migration are explicitly marked legacy_unknown:
-- their prior job decisions cannot be reconstructed safely, so retries must
-- preserve the ambiguity instead of scheduling replacements.

ALTER TABLE session_summaries
    ADD COLUMN followup_scheduling_completed_at_epoch INTEGER;

ALTER TABLE session_summaries
    ADD COLUMN followup_scheduling_state TEXT
    CHECK (followup_scheduling_state IS NULL OR followup_scheduling_state IN (
        'claimed', 'completed', 'legacy_unknown'
    ));

ALTER TABLE session_summaries
    ADD COLUMN followup_compress_job_id INTEGER;

ALTER TABLE session_summaries
    ADD COLUMN followup_dream_disposition TEXT
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
