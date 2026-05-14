-- v008_observation_evidence: captured-event evidence metadata for observations.
-- Existing observation rows keep the legacy columns. Extraction tasks fill
-- these nullable columns so downstream candidate generation can trace source
-- events without relying on raw text duplication.

ALTER TABLE observations ADD COLUMN host_id INTEGER;
ALTER TABLE observations ADD COLUMN project_id INTEGER;
ALTER TABLE observations ADD COLUMN session_row_id INTEGER;
ALTER TABLE observations ADD COLUMN observation_type TEXT;
ALTER TABLE observations ADD COLUMN text TEXT;
ALTER TABLE observations ADD COLUMN evidence_event_ids TEXT;
ALTER TABLE observations ADD COLUMN confidence REAL;

CREATE INDEX IF NOT EXISTS idx_observations_session_evidence
    ON observations(session_row_id, evidence_event_ids);

CREATE INDEX IF NOT EXISTS idx_observations_project_type
    ON observations(project_id, observation_type, created_at_epoch DESC);
