-- Review-queue throughput (#683): durable per-candidate review metadata so
-- batch and single review outcomes record who/what initiated them, plus a
-- covering index for queue-age aggregates.
ALTER TABLE memory_candidates ADD COLUMN review_actor TEXT;
ALTER TABLE memory_candidates ADD COLUMN reviewed_at_epoch INTEGER;
ALTER TABLE memory_candidates ADD COLUMN review_action_source TEXT;
ALTER TABLE memory_candidates ADD COLUMN review_batch_id TEXT;
ALTER TABLE memory_candidates ADD COLUMN review_reason TEXT;

CREATE INDEX IF NOT EXISTS idx_memory_candidates_review_status_created
    ON memory_candidates(review_status, created_at_epoch);
