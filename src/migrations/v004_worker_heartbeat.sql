CREATE TABLE IF NOT EXISTS worker_heartbeats (
    owner TEXT PRIMARY KEY,
    pid INTEGER,
    started_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_worker_heartbeats_updated
ON worker_heartbeats(updated_at_epoch DESC);
