-- v083_retrieval_enrichment_budget: stop upgrade-time AI stampedes.
--
-- Historical incomplete rows keep their deterministic search_context but are
-- deferred from automatic AI work. Rows inserted after this migration use the
-- pending default, and canonical source changes explicitly restore pending.

ALTER TABLE memories ADD COLUMN search_context_enrichment_state TEXT NOT NULL
    DEFAULT 'pending'
    CHECK (search_context_enrichment_state IN ('pending', 'ready', 'deferred', 'exhausted'));

UPDATE memories SET
    search_context_enrichment_state = CASE
        WHEN search_context_enrichment_version >= 1
         AND search_context_security_policy_version >= 1
         AND search_context_source_hash IS NOT NULL
        THEN 'ready'
        ELSE 'deferred'
    END,
    search_context_lease_owner = NULL,
    search_context_lease_expires_at_epoch = NULL,
    search_context_claimed_source_hash = NULL,
    search_context_claimed_enrichment_version = NULL,
    search_context_claimed_security_policy_version = NULL;

CREATE INDEX idx_memories_retrieval_enrichment_due
    ON memories(search_context_enrichment_state,
                search_context_next_retry_at_epoch,
                search_context_lease_expires_at_epoch,
                updated_at_epoch,
                id);

-- GPT-5.6 Codex subscription models are billed in credits, not by the generic
-- GPT-5 USD fallback. Preserve token counts and explicit operator overrides,
-- but remove false remem-static USD estimates.
UPDATE ai_usage_events SET
    estimated_cost_usd = 0.0,
    pricing_source = 'unknown_pricing'
WHERE (lower(COALESCE(model, '')) LIKE '%gpt-5.6-luna%'
       OR lower(COALESCE(model, '')) LIKE '%gpt-5.6-sol%'
       OR lower(COALESCE(model, '')) LIKE '%gpt-5.6-terra%')
  AND pricing_source IN ('remem_static', 'remem_static_backfill');

-- Keep the v072 single-trigger convergence ordering while adding the state
-- transition. SQLite sibling-trigger ordering is undefined, so this remains
-- one trigger whose final FTS rebuild reads the persisted row.
DROP TRIGGER IF EXISTS memories_au;
CREATE TRIGGER memories_au
AFTER UPDATE OF title, content, memory_type, topic_key, files, search_context,
    search_context_fallback_source_hash ON memories
BEGIN
    UPDATE memories SET
        search_context = '',
        search_context_enrichment_state = 'pending',
        search_context_enrichment_version = 0,
        search_context_security_policy_version = (
            SELECT min_security_policy_version
            FROM retrieval_enrichment_compatibility WHERE id = 1
        ),
        search_context_source_hash = NULL,
        search_context_fallback_source_hash = NULL,
        search_context_index_hash = NULL,
        search_context_lease_owner = NULL,
        search_context_lease_expires_at_epoch = NULL,
        search_context_claimed_source_hash = NULL,
        search_context_claimed_enrichment_version = NULL,
        search_context_claimed_security_policy_version = NULL,
        search_context_failure_count = 0,
        search_context_next_retry_at_epoch = NULL,
        search_context_last_error_code = NULL
    WHERE id = new.id
      AND (new.title IS NOT old.title
           OR new.content IS NOT old.content
           OR new.memory_type IS NOT old.memory_type
           OR new.topic_key IS NOT old.topic_key
           OR new.files IS NOT old.files)
      AND new.search_context_fallback_source_hash IS old.search_context_fallback_source_hash;
    DELETE FROM memory_embeddings
    WHERE memory_id = new.id
      AND (new.title IS NOT old.title
           OR new.content IS NOT old.content
           OR new.memory_type IS NOT old.memory_type
           OR new.topic_key IS NOT old.topic_key
           OR new.files IS NOT old.files)
      AND new.search_context_fallback_source_hash IS old.search_context_fallback_source_hash;
    INSERT INTO memories_fts(memories_fts, rowid, title, content, search_context)
    VALUES ('delete', old.id, old.title, old.content, COALESCE(old.search_context, ''));
    INSERT INTO memories_fts(rowid, title, content, search_context)
    SELECT id, title, content, COALESCE(search_context, '')
    FROM memories WHERE id = new.id;
END;
