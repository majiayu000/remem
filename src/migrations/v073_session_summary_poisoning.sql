-- v073_session_summary_poisoning: durable poisoning verdict metadata on
-- session summaries (GH-855).
--
-- New summary writers must run the shared instruction-pattern verdict before
-- persisting and write an explicit 'safe' or 'quarantined' status. Rows that
-- existed before this migration are marked 'legacy_unscanned' so every
-- model-visible reader re-scans them until a verdict is recorded.

ALTER TABLE session_summaries ADD COLUMN poisoning_status TEXT NOT NULL
    DEFAULT 'safe'
    CHECK(poisoning_status IN ('legacy_unscanned', 'safe', 'quarantined', 'acknowledged'));
ALTER TABLE session_summaries ADD COLUMN quarantine_stage TEXT
    CHECK(quarantine_stage IN ('source', 'generated'));
ALTER TABLE session_summaries ADD COLUMN quarantine_field TEXT;
ALTER TABLE session_summaries ADD COLUMN quarantine_event_id INTEGER;
ALTER TABLE session_summaries ADD COLUMN quarantine_pattern_id TEXT;
ALTER TABLE session_summaries ADD COLUMN quarantine_pattern_version INTEGER;
ALTER TABLE session_summaries ADD COLUMN acknowledged_pattern_id TEXT;
ALTER TABLE session_summaries ADD COLUMN acknowledged_pattern_version INTEGER;
ALTER TABLE session_summaries ADD COLUMN acknowledged_at_epoch INTEGER;
ALTER TABLE session_summaries ADD COLUMN poisoning_block_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE session_summaries ADD COLUMN poisoning_last_blocked_at_epoch INTEGER;

-- Rows created before this migration have never been through the verdict.
UPDATE session_summaries SET poisoning_status = 'legacy_unscanned';

CREATE INDEX idx_session_summaries_poisoning
    ON session_summaries(poisoning_status);
