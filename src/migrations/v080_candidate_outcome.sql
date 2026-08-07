-- GH-958: persist the extraction-declared lesson outcome (success/failure) on
-- memory candidates so the review/auto-promote round-trip through the database
-- keeps it until promotion writes memory_lessons.outcome_kind.
ALTER TABLE memory_candidates ADD COLUMN outcome TEXT
    CHECK (outcome IS NULL OR outcome IN ('success', 'failure'));
