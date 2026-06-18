-- v047_lesson_outcome_metadata: explicit outcome signals for lesson memories.
--
-- Failure-trajectory distillation will be a later feeder. This migration only
-- gives lesson memories durable, constrained fields that can distinguish
-- unknown lessons from lessons learned from failures, recoveries, corrections,
-- reverts, or successful patterns.

ALTER TABLE memory_lessons
    ADD COLUMN outcome_kind TEXT NOT NULL DEFAULT 'unknown'
    CHECK (outcome_kind IN ('unknown', 'success', 'failure', 'recovery', 'correction', 'revert'));

ALTER TABLE memory_lessons
    ADD COLUMN success_count INTEGER NOT NULL DEFAULT 0
    CHECK (success_count >= 0);

ALTER TABLE memory_lessons
    ADD COLUMN failure_count INTEGER NOT NULL DEFAULT 0
    CHECK (failure_count >= 0);

ALTER TABLE memory_lessons
    ADD COLUMN recovery_count INTEGER NOT NULL DEFAULT 0
    CHECK (recovery_count >= 0);

ALTER TABLE memory_lessons
    ADD COLUMN correction_count INTEGER NOT NULL DEFAULT 0
    CHECK (correction_count >= 0);

ALTER TABLE memory_lessons
    ADD COLUMN revert_count INTEGER NOT NULL DEFAULT 0
    CHECK (revert_count >= 0);

CREATE INDEX IF NOT EXISTS idx_memory_lessons_outcome
    ON memory_lessons(outcome_kind, failure_count DESC, correction_count DESC, last_reinforced_at_epoch DESC);
