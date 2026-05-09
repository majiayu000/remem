ALTER TABLE pending_observations
ADD COLUMN host TEXT NOT NULL DEFAULT 'unknown';

ALTER TABLE jobs
ADD COLUMN host TEXT NOT NULL DEFAULT 'unknown';

CREATE INDEX IF NOT EXISTS idx_pending_identity_claim
ON pending_observations(host, project, session_id, status, next_retry_epoch, lease_expires_epoch, id);

CREATE INDEX IF NOT EXISTS idx_jobs_identity_state
ON jobs(host, project, session_id, job_type, state, created_at_epoch DESC);
