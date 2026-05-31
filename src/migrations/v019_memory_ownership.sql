-- v019_memory_ownership: explicit ownership/routing metadata for memory
-- governance. Fields are nullable during the compatibility period; future
-- routing/promotion work will enforce them for new active rows.

ALTER TABLE memories ADD COLUMN source_project TEXT;
ALTER TABLE memories ADD COLUMN target_project TEXT;
ALTER TABLE memories ADD COLUMN owner_scope TEXT;
ALTER TABLE memories ADD COLUMN owner_key TEXT;
ALTER TABLE memories ADD COLUMN topic_domain TEXT;
ALTER TABLE memories ADD COLUMN routing_confidence REAL;
ALTER TABLE memories ADD COLUMN routing_reason TEXT;
ALTER TABLE memories ADD COLUMN context_class TEXT;
ALTER TABLE memories ADD COLUMN expires_at_epoch INTEGER;
ALTER TABLE memories ADD COLUMN valid_from_epoch INTEGER;
ALTER TABLE memories ADD COLUMN valid_to_epoch INTEGER;

UPDATE memories
SET source_project = COALESCE(source_project, project),
    target_project = CASE
        WHEN COALESCE(scope, 'project') = 'global' THEN NULL
        ELSE COALESCE(target_project, project)
    END,
    owner_scope = COALESCE(
        owner_scope,
        CASE
            WHEN COALESCE(scope, 'project') = 'global' THEN 'user'
            ELSE 'repo'
        END
    ),
    owner_key = COALESCE(
        owner_key,
        CASE
            WHEN COALESCE(scope, 'project') = 'global' THEN 'user:default'
            ELSE project
        END
    ),
    context_class = COALESCE(context_class, 'startup_core');

ALTER TABLE memory_candidates ADD COLUMN source_project TEXT;
ALTER TABLE memory_candidates ADD COLUMN target_project TEXT;
ALTER TABLE memory_candidates ADD COLUMN owner_scope TEXT;
ALTER TABLE memory_candidates ADD COLUMN owner_key TEXT;
ALTER TABLE memory_candidates ADD COLUMN topic_domain TEXT;
ALTER TABLE memory_candidates ADD COLUMN routing_confidence REAL;
ALTER TABLE memory_candidates ADD COLUMN routing_reason TEXT;
ALTER TABLE memory_candidates ADD COLUMN context_class TEXT;
ALTER TABLE memory_candidates ADD COLUMN expires_at_epoch INTEGER;
ALTER TABLE memory_candidates ADD COLUMN valid_from_epoch INTEGER;
ALTER TABLE memory_candidates ADD COLUMN valid_to_epoch INTEGER;

UPDATE memory_candidates
SET source_project = COALESCE(
        source_project,
        (SELECT p.project_path FROM projects p WHERE p.id = memory_candidates.project_id)
    ),
    target_project = CASE
        WHEN scope = 'global' THEN NULL
        ELSE COALESCE(
            target_project,
            (SELECT p.project_path FROM projects p WHERE p.id = memory_candidates.project_id)
        )
    END,
    owner_scope = COALESCE(
        owner_scope,
        CASE
            WHEN scope = 'global' THEN 'user'
            WHEN project_id IS NOT NULL THEN 'repo'
            ELSE NULL
        END
    ),
    owner_key = COALESCE(
        owner_key,
        CASE
            WHEN scope = 'global' THEN 'user:default'
            ELSE (SELECT p.project_path FROM projects p WHERE p.id = memory_candidates.project_id)
        END
    );

ALTER TABLE workstreams ADD COLUMN source_project TEXT;
ALTER TABLE workstreams ADD COLUMN target_project TEXT;
ALTER TABLE workstreams ADD COLUMN owner_scope TEXT;
ALTER TABLE workstreams ADD COLUMN owner_key TEXT;
ALTER TABLE workstreams ADD COLUMN topic_domain TEXT;
ALTER TABLE workstreams ADD COLUMN routing_confidence REAL;
ALTER TABLE workstreams ADD COLUMN routing_reason TEXT;
ALTER TABLE workstreams ADD COLUMN context_class TEXT;
ALTER TABLE workstreams ADD COLUMN expires_at_epoch INTEGER;
ALTER TABLE workstreams ADD COLUMN valid_from_epoch INTEGER;
ALTER TABLE workstreams ADD COLUMN valid_to_epoch INTEGER;

UPDATE workstreams
SET source_project = COALESCE(source_project, project),
    target_project = COALESCE(target_project, project),
    owner_scope = COALESCE(owner_scope, 'repo'),
    owner_key = COALESCE(owner_key, project),
    context_class = COALESCE(context_class, 'startup_core');

ALTER TABLE session_summaries ADD COLUMN source_project TEXT;
ALTER TABLE session_summaries ADD COLUMN target_project TEXT;
ALTER TABLE session_summaries ADD COLUMN owner_scope TEXT;
ALTER TABLE session_summaries ADD COLUMN owner_key TEXT;
ALTER TABLE session_summaries ADD COLUMN topic_domain TEXT;
ALTER TABLE session_summaries ADD COLUMN routing_confidence REAL;
ALTER TABLE session_summaries ADD COLUMN routing_reason TEXT;
ALTER TABLE session_summaries ADD COLUMN context_class TEXT;
ALTER TABLE session_summaries ADD COLUMN expires_at_epoch INTEGER;
ALTER TABLE session_summaries ADD COLUMN valid_from_epoch INTEGER;
ALTER TABLE session_summaries ADD COLUMN valid_to_epoch INTEGER;

UPDATE session_summaries
SET source_project = COALESCE(
        source_project,
        (SELECT p.project_path FROM projects p WHERE p.id = session_summaries.project_id),
        project
    ),
    context_class = COALESCE(context_class, 'search_only');

CREATE INDEX IF NOT EXISTS idx_memories_owner_status
    ON memories(owner_scope, owner_key, status, updated_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_memories_source_project
    ON memories(source_project, updated_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_memories_target_project_status
    ON memories(target_project, status, updated_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_memory_candidates_owner_review
    ON memory_candidates(owner_scope, owner_key, review_status, updated_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_workstreams_owner_status
    ON workstreams(owner_scope, owner_key, status, updated_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_session_summaries_owner_created
    ON session_summaries(owner_scope, owner_key, created_at_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_session_summaries_source_project
    ON session_summaries(source_project, created_at_epoch DESC);
