-- v032_candidate_block_reason: persist why a candidate was not auto-promoted
-- so promotion hit rate can be aggregated instead of only logged.

ALTER TABLE memory_candidates ADD COLUMN auto_promote_block_reason TEXT;
