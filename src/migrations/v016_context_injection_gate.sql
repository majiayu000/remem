-- v016_context_injection_gate: Track context hook emissions so hosts with
-- task-scoped SessionStart hooks can avoid injecting duplicate startup context
-- into one conversation history.

CREATE TABLE IF NOT EXISTS context_injections (
    id INTEGER PRIMARY KEY,
    host TEXT NOT NULL,
    project TEXT NOT NULL,
    injection_key TEXT NOT NULL,
    session_id TEXT,
    transcript_path TEXT,
    context_hash TEXT NOT NULL,
    output_mode TEXT NOT NULL,
    output_chars INTEGER NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    last_emitted_epoch INTEGER NOT NULL,
    emit_count INTEGER NOT NULL DEFAULT 1,
    suppress_count INTEGER NOT NULL DEFAULT 0,
    UNIQUE(host, injection_key)
);

CREATE INDEX IF NOT EXISTS idx_context_injections_project_seen
    ON context_injections(project, updated_at_epoch DESC);
