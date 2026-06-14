-- v042_reference_time_epoch: store episode/source event time separately from
-- transaction/write time for ingest, extraction, and temporal provenance.

ALTER TABLE captured_events ADD COLUMN reference_time_epoch INTEGER;
UPDATE captured_events
SET reference_time_epoch = created_at_epoch
WHERE reference_time_epoch IS NULL;
CREATE INDEX IF NOT EXISTS idx_captured_events_project_reference_time
    ON captured_events(project_id, reference_time_epoch DESC);

ALTER TABLE observations ADD COLUMN reference_time_epoch INTEGER;
UPDATE observations
SET reference_time_epoch = created_at_epoch
WHERE reference_time_epoch IS NULL;
CREATE INDEX IF NOT EXISTS idx_observations_project_reference_time
    ON observations(project_id, reference_time_epoch DESC);

ALTER TABLE memories ADD COLUMN reference_time_epoch INTEGER;
UPDATE memories
SET reference_time_epoch = created_at_epoch
WHERE reference_time_epoch IS NULL;
CREATE INDEX IF NOT EXISTS idx_memories_project_reference_time
    ON memories(project, reference_time_epoch DESC);
