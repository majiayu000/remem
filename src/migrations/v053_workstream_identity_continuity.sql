-- v053_workstream_identity_continuity: preserve canonical workstream identity
-- across title drift and keep prior titles as alias/history evidence.

ALTER TABLE workstreams ADD COLUMN identity_key TEXT;
ALTER TABLE workstreams ADD COLUMN merged_into_workstream_id INTEGER REFERENCES workstreams(id);

CREATE TABLE IF NOT EXISTS workstream_aliases (
    id INTEGER PRIMARY KEY,
    workstream_id INTEGER NOT NULL REFERENCES workstreams(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    normalized_title TEXT NOT NULL,
    first_seen_epoch INTEGER NOT NULL,
    last_seen_epoch INTEGER NOT NULL,
    UNIQUE(workstream_id, normalized_title)
);

CREATE TABLE IF NOT EXISTS workstream_alias_sources (
    id INTEGER PRIMARY KEY,
    alias_id INTEGER NOT NULL REFERENCES workstream_aliases(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    memory_session_id TEXT,
    source_workstream_id INTEGER REFERENCES workstreams(id),
    observed_title TEXT NOT NULL,
    first_seen_epoch INTEGER NOT NULL,
    last_seen_epoch INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_workstream_aliases_lookup
    ON workstream_aliases(normalized_title);

CREATE INDEX IF NOT EXISTS idx_workstream_alias_sources_alias
    ON workstream_alias_sources(alias_id);

CREATE INDEX IF NOT EXISTS idx_workstreams_identity_key
    ON workstreams(identity_key);

CREATE INDEX IF NOT EXISTS idx_workstreams_merged_into
    ON workstreams(merged_into_workstream_id);

UPDATE workstreams
SET identity_key = 'ws_' || id
WHERE identity_key IS NULL;

WITH alias_input AS (
    SELECT
        id,
        title,
        lower(trim(title)) AS normalized_title,
        created_at_epoch,
        updated_at_epoch
    FROM workstreams
    WHERE title IS NOT NULL AND trim(title) <> ''
),
whitespace_pass AS (
    SELECT
        id,
        title,
        replace(replace(replace(replace(replace(normalized_title,
            char(9), ' '), char(10), ' '), char(11), ' '), char(12), ' '), char(13), ' ') AS normalized_title,
        created_at_epoch,
        updated_at_epoch
    FROM alias_input
),
separator_pass_1 AS (
    SELECT
        id,
        title,
        replace(replace(replace(replace(replace(normalized_title,
            '/', ' '), '\', ' '), '-', ' '), '_', ' '), ':', ' ') AS normalized_title,
        created_at_epoch,
        updated_at_epoch
    FROM whitespace_pass
),
separator_pass_2 AS (
    SELECT
        id,
        title,
        replace(replace(replace(replace(replace(replace(normalized_title,
            ';', ' '), ',', ' '), '.', ' '), '(', ' '), ')', ' '), '[', ' ') AS normalized_title,
        created_at_epoch,
        updated_at_epoch
    FROM separator_pass_1
),
separator_pass_3 AS (
    SELECT
        id,
        title,
        replace(replace(replace(replace(replace(replace(normalized_title,
            ']', ' '), '{', ' '), '}', ' '), '<', ' '), '>', ' '), '|', ' ') AS normalized_title,
        created_at_epoch,
        updated_at_epoch
    FROM separator_pass_2
),
space_pass_1 AS (
    SELECT
        id,
        title,
        replace(replace(replace(replace(normalized_title,
            '  ', ' '), '  ', ' '), '  ', ' '), '  ', ' ') AS normalized_title,
        created_at_epoch,
        updated_at_epoch
    FROM separator_pass_3
),
space_pass_2 AS (
    SELECT
        id,
        title,
        replace(replace(replace(replace(normalized_title,
            '  ', ' '), '  ', ' '), '  ', ' '), '  ', ' ') AS normalized_title,
        created_at_epoch,
        updated_at_epoch
    FROM space_pass_1
)
INSERT OR IGNORE INTO workstream_aliases (
    workstream_id,
    title,
    normalized_title,
    first_seen_epoch,
    last_seen_epoch
)
SELECT
    id,
    title,
    trim(normalized_title),
    created_at_epoch,
    updated_at_epoch
FROM space_pass_2;

INSERT INTO workstream_alias_sources (
    alias_id,
    source,
    memory_session_id,
    source_workstream_id,
    observed_title,
    first_seen_epoch,
    last_seen_epoch
)
SELECT
    wa.id,
    'migration',
    NULL,
    ws.id,
    ws.title,
    ws.created_at_epoch,
    ws.updated_at_epoch
FROM workstreams ws
JOIN workstream_aliases wa
  ON wa.workstream_id = ws.id
 AND wa.title = ws.title
WHERE ws.title IS NOT NULL AND trim(ws.title) <> '';
