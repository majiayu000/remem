-- v072_memory_retrieval_enrichment: index-only contextual enrichment identity (GH-850).
--
-- Adds enrichment identity/claim/lease/failure metadata to `memories`, the
-- `retrieval_enrichment_compatibility` singleton with a monotonic security
-- policy floor, and rebuilds `memories_au` so that:
--   1. a raw canonical UPDATE (title/content/memory_type/topic_key/files
--      changed without the writer resetting the fallback source hash) persists
--      an empty deterministic fallback plus invalidated enrichment identity in
--      the same outer transaction, and drops now-stale vectors so the existing
--      missing-vector backfill rebuilds them;
--   2. every FTS rebuild reads the FINAL persisted row via SELECT instead of
--      trusting OLD/NEW images, so a later unrelated UPDATE can never
--      re-introduce stale enrichment text into the index.
--
-- This migration is O(1) additive DDL only: no AI calls, no full-table
-- backfill. Historical rows keep their deterministic search_context and are
-- upgraded progressively by the idle worker lane.

ALTER TABLE memories ADD COLUMN search_context_enrichment_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN search_context_security_policy_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN search_context_source_hash TEXT;
ALTER TABLE memories ADD COLUMN search_context_fallback_source_hash TEXT;
ALTER TABLE memories ADD COLUMN search_context_index_hash TEXT;
ALTER TABLE memories ADD COLUMN search_context_enrichment_attempt INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN search_context_lease_owner TEXT;
ALTER TABLE memories ADD COLUMN search_context_lease_expires_at_epoch INTEGER;
ALTER TABLE memories ADD COLUMN search_context_claimed_source_hash TEXT;
ALTER TABLE memories ADD COLUMN search_context_claimed_enrichment_version INTEGER;
ALTER TABLE memories ADD COLUMN search_context_claimed_security_policy_version INTEGER;
ALTER TABLE memories ADD COLUMN search_context_failure_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN search_context_next_retry_at_epoch INTEGER;
ALTER TABLE memories ADD COLUMN search_context_last_error_code TEXT;

CREATE TABLE retrieval_enrichment_compatibility (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    min_security_policy_version INTEGER NOT NULL,
    compatibility_epoch INTEGER NOT NULL,
    target_security_policy_version INTEGER NOT NULL,
    convergence_state TEXT NOT NULL CHECK (convergence_state IN ('ready', 'rebuilding')),
    updated_at_epoch INTEGER NOT NULL
);

INSERT INTO retrieval_enrichment_compatibility
    (id, min_security_policy_version, compatibility_epoch,
     target_security_policy_version, convergence_state, updated_at_epoch)
VALUES (1, 1, 1, 1, 'ready', strftime('%s', 'now'));

-- The floor/epoch are monotonic: no downgrade, and every floor/target/state
-- change must strictly increase the epoch. Deleting the singleton is refused
-- permanently so a delete+reinsert cannot bypass monotonicity.
CREATE TRIGGER retrieval_enrichment_compatibility_bu
BEFORE UPDATE ON retrieval_enrichment_compatibility
BEGIN
    SELECT CASE
        WHEN new.min_security_policy_version < old.min_security_policy_version
            THEN RAISE(ABORT, 'retrieval enrichment security policy floor must not decrease')
        WHEN new.compatibility_epoch < old.compatibility_epoch
            THEN RAISE(ABORT, 'retrieval enrichment compatibility epoch must not decrease')
        WHEN (new.min_security_policy_version != old.min_security_policy_version
              OR new.target_security_policy_version != old.target_security_policy_version
              OR new.convergence_state != old.convergence_state)
             AND new.compatibility_epoch <= old.compatibility_epoch
            THEN RAISE(ABORT, 'retrieval enrichment compatibility change requires a strictly increasing epoch')
    END;
END;

CREATE TRIGGER retrieval_enrichment_compatibility_bd
BEFORE DELETE ON retrieval_enrichment_compatibility
BEGIN
    SELECT RAISE(ABORT, 'retrieval enrichment compatibility singleton must not be deleted');
END;

-- Rebuild the AFTER UPDATE trigger from v070 as a single trigger so the
-- convergence UPDATE always runs before the FTS rebuild (sibling trigger
-- ordering is undefined in SQLite; one body makes the sequence deterministic).
-- The OF-list keeps v070's guarantee that internal metadata writes (e.g. the
-- web-console version bump) never issue a second external-content delete, and
-- adds the canonical source columns so bypass writes converge. PRAGMA
-- recursive_triggers stays off (the default), so the inner UPDATE does not
-- re-fire this trigger; the trailing FTS rebuild reads the persisted row.
DROP TRIGGER IF EXISTS memories_au;
CREATE TRIGGER memories_au
AFTER UPDATE OF title, content, memory_type, topic_key, files, search_context,
    search_context_fallback_source_hash ON memories
BEGIN
    UPDATE memories SET
        search_context = '',
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
