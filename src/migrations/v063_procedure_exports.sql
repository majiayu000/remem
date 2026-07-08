-- v063_procedure_exports: registry for review-gated procedure draft exports.

CREATE TABLE IF NOT EXISTS procedure_exports (
    id INTEGER PRIMARY KEY,
    memory_id INTEGER NOT NULL,
    project TEXT NOT NULL,
    format TEXT NOT NULL CHECK (format IN ('claude-skill', 'codex-prompt', 'runbook-md')),
    output_path TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    source_digest TEXT NOT NULL,
    source_digest_version INTEGER NOT NULL,
    source_updated_at_epoch INTEGER NOT NULL,
    exported_at_epoch INTEGER NOT NULL,
    remem_version TEXT NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    UNIQUE(memory_id, format, output_path)
);

CREATE INDEX IF NOT EXISTS idx_procedure_exports_project
    ON procedure_exports(project, exported_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_procedure_exports_memory
    ON procedure_exports(memory_id, exported_at_epoch DESC);
