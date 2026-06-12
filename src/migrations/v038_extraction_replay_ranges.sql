-- v038_extraction_replay_ranges: durable replay ledger for exhausted
-- extraction task ranges. The primary task cursor can keep moving so new
-- events remain extractable while failed ranges stay visible and bounded.

CREATE TABLE IF NOT EXISTS extraction_replay_ranges (
    id INTEGER PRIMARY KEY,
    source_task_id INTEGER NOT NULL REFERENCES extraction_tasks(id),
    replay_task_id INTEGER REFERENCES extraction_tasks(id),
    task_kind TEXT NOT NULL,
    host_id INTEGER NOT NULL REFERENCES hosts(id),
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    session_row_id INTEGER REFERENCES sessions(id),
    from_event_id INTEGER NOT NULL,
    to_event_id INTEGER NOT NULL,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    UNIQUE(source_task_id, from_event_id, to_event_id)
);

ALTER TABLE extraction_tasks
ADD COLUMN replay_range_id INTEGER REFERENCES extraction_replay_ranges(id);

CREATE INDEX IF NOT EXISTS idx_extraction_replay_ranges_status
    ON extraction_replay_ranges(status, updated_at_epoch);

CREATE INDEX IF NOT EXISTS idx_extraction_replay_ranges_project
    ON extraction_replay_ranges(project_id, status, updated_at_epoch);

CREATE INDEX IF NOT EXISTS idx_extraction_tasks_replay_range
    ON extraction_tasks(replay_range_id, task_kind, status);
