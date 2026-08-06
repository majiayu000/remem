-- v077_dream_backfill: bind historical (pre-v076) Dream-merged memories to
-- their quarantine artifacts so review approval can restore them in place.
--
-- Forward-path artifacts (written by the v076 quarantine flow) leave
-- backfill_memory_id NULL. Stock-backfill artifacts always reference the exact
-- memories row they retired pending review.

ALTER TABLE dream_quarantine_artifacts
    ADD COLUMN backfill_memory_id INTEGER REFERENCES memories(id) ON DELETE RESTRICT;

-- A backfill binding only makes sense for merge payloads (the stock row is the
-- merged output); other decision kinds carry no generated fields to restore.
CREATE TRIGGER dream_quarantine_artifacts_backfill_merge_only
BEFORE INSERT ON dream_quarantine_artifacts
WHEN NEW.backfill_memory_id IS NOT NULL AND NEW.decision_kind != 'merge'
BEGIN
    SELECT RAISE(ABORT, 'Dream backfill artifact must be a merge decision');
END;

-- The binding is part of the immutable payload: once written it can never be
-- retargeted to a different memory.
CREATE TRIGGER dream_quarantine_artifacts_backfill_immutable
BEFORE UPDATE OF backfill_memory_id ON dream_quarantine_artifacts
BEGIN
    SELECT RAISE(ABORT, 'Dream quarantine artifact backfill binding is immutable');
END;

CREATE UNIQUE INDEX idx_dream_quarantine_backfill_memory
ON dream_quarantine_artifacts(backfill_memory_id)
WHERE backfill_memory_id IS NOT NULL;
