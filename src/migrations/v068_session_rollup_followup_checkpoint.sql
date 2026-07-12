-- v068_session_rollup_followup_checkpoint: record one durable Compress/Dream
-- scheduling decision for each persisted SessionRollup event range. Historical
-- rows remain NULL because their prior Dream decision cannot be reconstructed.

ALTER TABLE session_summaries
    ADD COLUMN followup_scheduling_completed_at_epoch INTEGER;
