-- v036_capture_audit_events: durable audit trail for capture inputs that do
-- not become captured_events.

CREATE TABLE IF NOT EXISTS capture_audit_events (
    id INTEGER PRIMARY KEY,
    created_at_epoch INTEGER NOT NULL,
    host TEXT,
    adapter TEXT,
    session_id TEXT,
    project TEXT,
    cwd TEXT,
    tool_name TEXT,
    reason TEXT NOT NULL,
    detail TEXT,
    content_hash TEXT,
    payload_preview TEXT
);

CREATE INDEX IF NOT EXISTS idx_capture_audit_reason_created
    ON capture_audit_events(reason, created_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_capture_audit_project_created
    ON capture_audit_events(project, created_at_epoch DESC);
