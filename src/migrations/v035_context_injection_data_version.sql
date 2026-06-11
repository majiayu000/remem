-- v035_context_injection_data_version: persist a conservative context data
-- fingerprint so strict SessionStart can suppress duplicate context before
-- paying full render cost.

ALTER TABLE context_injections ADD COLUMN data_version TEXT;

CREATE INDEX IF NOT EXISTS idx_workstreams_target_status
    ON workstreams(target_project, status, updated_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_session_summaries_target_created
    ON session_summaries(target_project, created_at_epoch DESC);
