-- v036_capture_drop_events: observable capture skip/drop ledger.

CREATE TABLE IF NOT EXISTS capture_drop_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    host_id TEXT,
    session_id TEXT,
    project TEXT,
    tool_name TEXT,
    reason TEXT NOT NULL,
    detail TEXT,
    spill_path TEXT,
    recovered_event_id INTEGER REFERENCES captured_events(id) ON DELETE SET NULL,
    created_at_epoch INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    recovered_at_epoch INTEGER
);

CREATE INDEX IF NOT EXISTS idx_capture_drop_events_reason_time
    ON capture_drop_events(reason, created_at_epoch);

CREATE INDEX IF NOT EXISTS idx_capture_drop_events_unrecovered_spill
    ON capture_drop_events(reason, recovered_event_id)
    WHERE reason = 'db_open_failed';
