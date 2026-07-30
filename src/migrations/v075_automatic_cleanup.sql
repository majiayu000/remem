-- v074_automatic_cleanup: durable automatic-maintenance scheduling, run
-- history, and fail-closed retention metadata for the legacy events table.

ALTER TABLE events
ADD COLUMN retention_class TEXT NOT NULL DEFAULT 'audit'
    CHECK (retention_class IN ('ephemeral', 'audit'));

-- Only event kinds with an established low-level/tooling lifecycle are
-- eligible for time-based deletion. Unknown and governance event kinds stay
-- audit-class by default.
UPDATE events
SET retention_class = 'ephemeral'
WHERE event_type IN (
    'file_edit',
    'file_create',
    'bash',
    'search',
    'agent',
    'tool_result',
    'cursor_tool_failure'
);

CREATE INDEX idx_events_retention_created
ON events(retention_class, created_at_epoch);

-- Both cleanup's anti-join and the delete-protection trigger probe this audit
-- reference for every event candidate. Keep the lookup indexed so the atomic
-- maintenance transaction does not degrade to an events × mutation-ledger scan.
CREATE INDEX idx_api_mutation_requests_audit
ON api_mutation_requests(audit_id);

-- Keep the once-daily large-store pass on indexed status/time ranges. These
-- indexes bound read amplification before any destructive predicate is
-- revalidated inside the cleanup transaction.
CREATE INDEX idx_memories_cleanup_expiry
ON memories(status, expires_at_epoch)
WHERE expires_at_epoch IS NOT NULL;

CREATE INDEX idx_memories_cleanup_archive
ON memories(status, updated_at_epoch);

CREATE INDEX idx_workstreams_cleanup_inactivity
ON workstreams(status, updated_at_epoch);

CREATE INDEX idx_observations_cleanup_sources
ON observations(status, created_at_epoch, id);

CREATE INDEX idx_compressed_sources_cleanup_age
ON compressed_observation_sources(source_observation_id, created_at_epoch);

-- api_mutation_requests.audit_id is deliberately not a foreign key because
-- the web idempotency ledger is append-only. Preserve its referenced audit
-- event explicitly so cleanup cannot invalidate restore provenance.
CREATE TRIGGER events_preserve_api_mutation_audit
BEFORE DELETE ON events
WHEN EXISTS (
    SELECT 1
    FROM api_mutation_requests
    WHERE audit_id = OLD.id
)
BEGIN
    SELECT RAISE(ABORT, 'cannot delete event referenced by api_mutation_requests.audit_id');
END;

CREATE TABLE maintenance_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id INTEGER REFERENCES jobs(id) ON DELETE SET NULL,
    "trigger" TEXT NOT NULL
        CHECK ("trigger" IN ('automatic', 'manual')),
    policy_version INTEGER NOT NULL
        CHECK (policy_version > 0),
    started_at_epoch INTEGER NOT NULL
        CHECK (started_at_epoch >= 0),
    finished_at_epoch INTEGER NOT NULL
        CHECK (finished_at_epoch >= started_at_epoch),
    outcome TEXT NOT NULL
        CHECK (outcome IN ('success', 'failure')),
    counts_json TEXT,
    error TEXT,
    CHECK (
        (
            outcome = 'success'
            AND counts_json IS NOT NULL
            AND length(trim(counts_json)) > 0
            AND json_valid(counts_json)
            AND error IS NULL
        )
        OR
        (
            outcome = 'failure'
            AND counts_json IS NULL
            AND error IS NOT NULL
            AND length(trim(error)) > 0
        )
    )
);

CREATE INDEX idx_maintenance_runs_trigger_outcome_finished
ON maintenance_runs("trigger", outcome, finished_at_epoch DESC, id DESC);

CREATE INDEX idx_maintenance_runs_job
ON maintenance_runs(job_id)
WHERE job_id IS NOT NULL;

-- Cleanup owns one global active slot. Rebuild the ordinary identity index so
-- cleanup is not accidentally scoped by host/project/session.
DROP INDEX idx_jobs_active_ordinary_unique;

CREATE UNIQUE INDEX idx_jobs_active_ordinary_unique
ON jobs(host, job_type, project, COALESCE(session_id, ''))
WHERE job_type NOT IN ('dream', 'compile_rules', 'cleanup')
  AND state IN ('pending', 'processing');

CREATE UNIQUE INDEX idx_jobs_active_cleanup_unique
ON jobs(job_type)
WHERE job_type = 'cleanup'
  AND state IN ('pending', 'processing');
