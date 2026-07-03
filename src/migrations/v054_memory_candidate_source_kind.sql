-- v054_memory_candidate_source_kind: distinguish observation, summary, and
-- legacy unattributed memory candidates for promotion diagnostics.

ALTER TABLE memory_candidates
    ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'unattributed';

CREATE INDEX IF NOT EXISTS idx_memory_candidates_source_review
    ON memory_candidates(source_kind, review_status, created_at_epoch DESC);
