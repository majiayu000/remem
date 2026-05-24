-- v014_procedure_verifications: persist verified procedure traces incrementally.

CREATE TABLE IF NOT EXISTS procedure_verifications (
    id INTEGER PRIMARY KEY,
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_row_id INTEGER NOT NULL REFERENCES sessions(id),
    branch TEXT,
    workflow_key TEXT NOT NULL,
    command TEXT NOT NULL,
    files_touched TEXT NOT NULL,
    source_event_id INTEGER NOT NULL REFERENCES captured_events(id),
    verified_at_epoch INTEGER NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    UNIQUE(host_id, project_id, session_row_id, source_event_id)
);

CREATE INDEX IF NOT EXISTS idx_procedure_verifications_lookup
    ON procedure_verifications(host_id, project_id, session_row_id, workflow_key, command, branch, verified_at_epoch);
CREATE INDEX IF NOT EXISTS idx_procedure_verifications_source
    ON procedure_verifications(source_event_id);
