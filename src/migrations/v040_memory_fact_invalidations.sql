-- v040_memory_fact_invalidations: record transaction-time invalidation for
-- temporal facts. `valid_to_epoch` remains event-validity time; this column
-- records when remem learned that the fact stopped being current.

ALTER TABLE memory_facts
    ADD COLUMN invalidated_at_epoch INTEGER;

-- Pre-v040 rows cannot recover the exact time remem invalidated them. For rows
-- already marked stale, updated_at_epoch is the best available transaction-time
-- approximation and prevents NULL from meaning both "active" and "unknown stale".
UPDATE memory_facts
SET invalidated_at_epoch = updated_at_epoch
WHERE status = 'stale'
  AND invalidated_at_epoch IS NULL;

CREATE INDEX IF NOT EXISTS idx_memory_facts_invalidated
    ON memory_facts(project, invalidated_at_epoch)
    WHERE invalidated_at_epoch IS NOT NULL;
