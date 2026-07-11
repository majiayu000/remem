-- v066_session_rollup_evidence_checkpoint: preserve the exact bounded,
-- redacted transcript evidence used by SessionRollup and record successful raw
-- archive completion so persisted-range retries do not depend on source files.

ALTER TABLE session_summaries
    ADD COLUMN transcript_evidence_json TEXT;

ALTER TABLE session_summaries
    ADD COLUMN raw_archive_completed_at_epoch INTEGER;
