-- v039_context_injection_items: append-only per-item audit rows for context
-- injection accountability. `context_injections` remains the output-level
-- de-dup gate; this table records which memory-like items were injected,
-- dropped, or abstained for each emission.

CREATE TABLE IF NOT EXISTS context_injection_items (
    id INTEGER PRIMARY KEY,
    injection_run_id TEXT NOT NULL,
    host TEXT NOT NULL,
    project TEXT NOT NULL,
    session_id TEXT,
    injection_key TEXT NOT NULL,
    hook_source TEXT,
    context_hash TEXT,
    output_mode TEXT NOT NULL,
    decision TEXT NOT NULL,
    item_kind TEXT NOT NULL,
    item_id INTEGER,
    memory_id INTEGER,
    channel TEXT NOT NULL,
    score REAL,
    render_order INTEGER,
    status TEXT NOT NULL CHECK (status IN ('injected', 'dropped', 'abstained')),
    drop_reason TEXT,
    title TEXT,
    provenance TEXT,
    staleness TEXT,
    injected_at_epoch INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_context_injection_items_session
    ON context_injection_items(host, session_id, injected_at_epoch, render_order);

CREATE INDEX IF NOT EXISTS idx_context_injection_items_memory
    ON context_injection_items(memory_id, injected_at_epoch DESC);

CREATE INDEX IF NOT EXISTS idx_context_injection_items_project
    ON context_injection_items(project, injected_at_epoch DESC);
