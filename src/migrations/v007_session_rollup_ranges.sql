-- v007_session_rollup_ranges: event-range metadata for session rollup tasks.
-- Existing session_summaries rows keep the legacy columns; captured-event
-- rollups use these nullable columns to make cursor advancement idempotent.

ALTER TABLE session_summaries ADD COLUMN host_id INTEGER;
ALTER TABLE session_summaries ADD COLUMN project_id INTEGER;
ALTER TABLE session_summaries ADD COLUMN session_row_id INTEGER;
ALTER TABLE session_summaries ADD COLUMN summary_text TEXT;
ALTER TABLE session_summaries ADD COLUMN covered_from_event_id INTEGER;
ALTER TABLE session_summaries ADD COLUMN covered_to_event_id INTEGER;
ALTER TABLE session_summaries ADD COLUMN model TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_session_summaries_event_range
    ON session_summaries(session_row_id, covered_from_event_id, covered_to_event_id)
    WHERE session_row_id IS NOT NULL
      AND covered_from_event_id IS NOT NULL
      AND covered_to_event_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_session_summaries_session_range
    ON session_summaries(session_row_id, covered_from_event_id);
