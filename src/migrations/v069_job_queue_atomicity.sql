-- v069_job_queue_atomicity: reconcile pre-existing active job duplicates and
-- enforce the ordinary, Dream, and CompileRules active identity contracts.

DROP TABLE IF EXISTS temp._v069_context;
DROP TABLE IF EXISTS temp._v069_survivors;
DROP TABLE IF EXISTS temp._v069_dream_pending_order;
DROP TABLE IF EXISTS temp._v069_dream_replay_result;
DROP TABLE IF EXISTS temp._v069_reconciled;
DROP TABLE IF EXISTS temp._v069_validation;

CREATE TEMP TABLE _v069_context (
    migration_now INTEGER NOT NULL
);
INSERT INTO _v069_context(migration_now)
VALUES (CAST(strftime('%s', 'now') AS INTEGER));

-- A pre-v069 process may have inserted a late active Summary row after v064.
-- Preserve v064's exact marker so failure and frozen-surface readers continue
-- to classify the row as retired rather than actionable work. Terminal Summary
-- history is intentionally outside this active-only predicate.
UPDATE jobs
SET state = 'failed',
    attempt_count = max(COALESCE(attempt_count, 0), COALESCE(max_attempts, attempt_count, 0)),
    next_retry_epoch = 0,
    last_error = 'legacy summary job rejected during GH684 summary retirement upgrade; SessionRollup owns session summary output',
    failure_class = 'permanent',
    failed_at_epoch = COALESCE(failed_at_epoch, (SELECT migration_now FROM _v069_context)),
    archived_at_epoch = NULL,
    lease_owner = NULL,
    lease_expires_epoch = NULL,
    updated_at_epoch = (SELECT migration_now FROM _v069_context)
WHERE job_type = 'summary'
  AND state IN ('pending', 'processing');

-- A NULL processing lease is neither current nor recoverable. Normalize only
-- the expiry so existing stuck/recovery readers immediately see it as expired.
UPDATE jobs
SET lease_expires_epoch = (SELECT migration_now - 1 FROM _v069_context)
WHERE job_type <> 'summary'
  AND state = 'processing'
  AND lease_expires_epoch IS NULL;

CREATE TEMP TABLE _v069_survivors (
    id INTEGER PRIMARY KEY,
    identity_kind TEXT NOT NULL
);

-- Ordinary identities prefer a processing row. A current lease wins over an
-- expired lease, then the latest lease/update/id wins. Pending-only groups use
-- the existing claim order.
INSERT INTO _v069_survivors(id, identity_kind)
SELECT id, 'ordinary'
FROM (
    SELECT
        id,
        ROW_NUMBER() OVER (
            PARTITION BY host, job_type, project, COALESCE(session_id, '')
            ORDER BY
                CASE WHEN state = 'processing' THEN 0 ELSE 1 END,
                CASE
                    WHEN state = 'processing'
                     AND lease_expires_epoch >= (SELECT migration_now FROM _v069_context)
                    THEN 0
                    WHEN state = 'processing' THEN 1
                    ELSE 0
                END,
                CASE WHEN state = 'processing' THEN lease_expires_epoch END DESC,
                CASE WHEN state = 'processing' THEN updated_at_epoch END DESC,
                CASE WHEN state = 'processing' THEN id END DESC,
                CASE WHEN state = 'pending' THEN priority END ASC,
                CASE WHEN state = 'pending' THEN created_at_epoch END ASC,
                CASE WHEN state = 'pending' THEN id END ASC
        ) AS survivor_rank
    FROM jobs
    WHERE job_type NOT IN ('dream', 'compile_rules')
      AND state IN ('pending', 'processing')
)
WHERE survivor_rank = 1;

-- Dream has one project-wide active slot. A processing row always wins; only
-- pending-only projects select the earliest pending row as the replay base.
INSERT INTO _v069_survivors(id, identity_kind)
SELECT id, 'dream'
FROM (
    SELECT
        id,
        project,
        ROW_NUMBER() OVER (
            PARTITION BY project
            ORDER BY
                CASE
                    WHEN lease_expires_epoch >= (SELECT migration_now FROM _v069_context)
                    THEN 0 ELSE 1
                END,
                lease_expires_epoch DESC,
                updated_at_epoch DESC,
                id DESC
        ) AS survivor_rank
    FROM jobs
    WHERE job_type = 'dream'
      AND state = 'processing'
)
WHERE survivor_rank = 1;

INSERT INTO _v069_survivors(id, identity_kind)
SELECT id, 'dream'
FROM (
    SELECT
        id,
        project,
        ROW_NUMBER() OVER (
            PARTITION BY project
            ORDER BY created_at_epoch ASC, id ASC
        ) AS survivor_rank
    FROM jobs AS pending_dream
    WHERE job_type = 'dream'
      AND state = 'pending'
      AND NOT EXISTS (
          SELECT 1
          FROM jobs AS processing_dream
          WHERE processing_dream.job_type = 'dream'
            AND processing_dream.project = pending_dream.project
            AND processing_dream.state = 'processing'
      )
)
WHERE survivor_rank = 1;

-- Replay pending-only Dream snapshots in stable creation order. The nested
-- json_valid/json_type CASE is deliberately fail-closed for malformed,
-- missing, non-string, and blank profile fields while preserving raw payloads.
CREATE TEMP TABLE _v069_dream_pending_order AS
SELECT
    id,
    project,
    host,
    payload_json,
    priority,
    ROW_NUMBER() OVER (
        PARTITION BY project
        ORDER BY created_at_epoch ASC, id ASC
    ) AS replay_rank,
    COUNT(*) OVER (PARTITION BY project) AS replay_count,
    CASE
        WHEN json_valid(payload_json) = 1 THEN
            CASE
                WHEN json_type(payload_json, '$.remem_ai_profile') = 'text'
                THEN trim(CAST(json_extract(payload_json, '$.remem_ai_profile') AS TEXT))
                ELSE ''
            END
        ELSE ''
    END AS profile_key
FROM jobs AS pending_dream
WHERE job_type = 'dream'
  AND state = 'pending'
  AND NOT EXISTS (
      SELECT 1
      FROM jobs AS processing_dream
      WHERE processing_dream.job_type = 'dream'
        AND processing_dream.project = pending_dream.project
        AND processing_dream.state = 'processing'
  );

CREATE TEMP TABLE _v069_dream_replay_result AS
WITH RECURSIVE dream_replay(
    project,
    replay_rank,
    replay_count,
    survivor_id,
    host,
    payload_json,
    priority,
    profile_key
) AS (
    SELECT
        project,
        replay_rank,
        replay_count,
        id,
        host,
        payload_json,
        priority,
        profile_key
    FROM _v069_dream_pending_order
    WHERE replay_rank = 1

    UNION ALL

    SELECT
        incoming.project,
        incoming.replay_rank,
        incoming.replay_count,
        current.survivor_id,
        CASE
            WHEN incoming.profile_key <> ''
             AND incoming.profile_key <> current.profile_key
            THEN incoming.host ELSE current.host
        END,
        CASE
            WHEN incoming.profile_key <> ''
             AND incoming.profile_key <> current.profile_key
            THEN incoming.payload_json ELSE current.payload_json
        END,
        CASE
            WHEN incoming.profile_key <> ''
             AND incoming.profile_key <> current.profile_key
            THEN min(current.priority, incoming.priority) ELSE current.priority
        END,
        CASE
            WHEN incoming.profile_key <> ''
             AND incoming.profile_key <> current.profile_key
            THEN incoming.profile_key ELSE current.profile_key
        END
    FROM dream_replay AS current
    JOIN _v069_dream_pending_order AS incoming
      ON incoming.project = current.project
     AND incoming.replay_rank = current.replay_rank + 1
)
SELECT survivor_id, host, payload_json, priority
FROM dream_replay
WHERE replay_rank = replay_count;

UPDATE jobs
SET host = (
        SELECT replay.host
        FROM _v069_dream_replay_result AS replay
        WHERE replay.survivor_id = jobs.id
    ),
    payload_json = (
        SELECT replay.payload_json
        FROM _v069_dream_replay_result AS replay
        WHERE replay.survivor_id = jobs.id
    ),
    priority = (
        SELECT replay.priority
        FROM _v069_dream_replay_result AS replay
        WHERE replay.survivor_id = jobs.id
    )
WHERE id IN (SELECT survivor_id FROM _v069_dream_replay_result);

-- CompileRules owns one project/state slot, preserving one processing row and
-- one pending successor when both exist.
INSERT INTO _v069_survivors(id, identity_kind)
SELECT id, 'compile_rules'
FROM (
    SELECT
        id,
        ROW_NUMBER() OVER (
            PARTITION BY project
            ORDER BY
                CASE
                    WHEN lease_expires_epoch >= (SELECT migration_now FROM _v069_context)
                    THEN 0 ELSE 1
                END,
                lease_expires_epoch DESC,
                updated_at_epoch DESC,
                id DESC
        ) AS survivor_rank
    FROM jobs
    WHERE job_type = 'compile_rules'
      AND state = 'processing'
)
WHERE survivor_rank = 1;

INSERT INTO _v069_survivors(id, identity_kind)
SELECT id, 'compile_rules'
FROM (
    SELECT
        id,
        ROW_NUMBER() OVER (
            PARTITION BY project
            ORDER BY priority ASC, created_at_epoch ASC, id ASC
        ) AS survivor_rank
    FROM jobs
    WHERE job_type = 'compile_rules'
      AND state = 'pending'
)
WHERE survivor_rank = 1;

-- Materialize every redundant row and its canonical id before changing state.
-- This table is intentionally retained until the Rust post-migration hook has
-- logged safe per-kind counts.
CREATE TEMP TABLE _v069_reconciled AS
SELECT
    duplicate_id,
    canonical_id,
    identity_kind,
    manual_review,
    printf(
        '[job_queue_atomicity_migration_duplicate duplicate_id=%d canonical_id=%d identity_kind=%s manual_review=%s]',
        duplicate_id,
        canonical_id,
        identity_kind,
        CASE manual_review WHEN 1 THEN 'true' ELSE 'false' END
    ) AS marker
FROM (
    SELECT
        duplicate.id AS duplicate_id,
        canonical.id AS canonical_id,
        'ordinary' AS identity_kind,
        CASE WHEN (
            SELECT COUNT(*)
            FROM jobs AS group_row
            WHERE group_row.host = duplicate.host
              AND group_row.job_type = duplicate.job_type
              AND group_row.project = duplicate.project
              AND COALESCE(group_row.session_id, '') = COALESCE(duplicate.session_id, '')
              AND group_row.state = 'processing'
        ) > 1 THEN 1 ELSE 0 END AS manual_review
    FROM jobs AS duplicate
    JOIN jobs AS canonical
      ON canonical.host = duplicate.host
     AND canonical.job_type = duplicate.job_type
     AND canonical.project = duplicate.project
     AND COALESCE(canonical.session_id, '') = COALESCE(duplicate.session_id, '')
    JOIN _v069_survivors AS survivor
      ON survivor.id = canonical.id
     AND survivor.identity_kind = 'ordinary'
    WHERE duplicate.job_type NOT IN ('dream', 'compile_rules')
      AND duplicate.state IN ('pending', 'processing')
      AND duplicate.id <> canonical.id

    UNION ALL

    SELECT
        duplicate.id,
        canonical.id,
        'dream',
        CASE WHEN (
            SELECT COUNT(*)
            FROM jobs AS group_row
            WHERE group_row.job_type = 'dream'
              AND group_row.project = duplicate.project
              AND group_row.state = 'processing'
        ) > 1 OR (
            SELECT COUNT(DISTINCT group_row.payload_json)
            FROM jobs AS group_row
            WHERE group_row.job_type = 'dream'
              AND group_row.project = duplicate.project
              AND group_row.state IN ('pending', 'processing')
        ) > 1 THEN 1 ELSE 0 END
    FROM jobs AS duplicate
    JOIN jobs AS canonical
      ON canonical.job_type = 'dream'
     AND canonical.project = duplicate.project
    JOIN _v069_survivors AS survivor
      ON survivor.id = canonical.id
     AND survivor.identity_kind = 'dream'
    WHERE duplicate.job_type = 'dream'
      AND duplicate.state IN ('pending', 'processing')
      AND duplicate.id <> canonical.id

    UNION ALL

    SELECT
        duplicate.id,
        canonical.id,
        'compile_rules',
        CASE WHEN (
            SELECT COUNT(*)
            FROM jobs AS group_row
            WHERE group_row.job_type = 'compile_rules'
              AND group_row.project = duplicate.project
              AND group_row.state = 'processing'
        ) > 1 THEN 1 ELSE 0 END
    FROM jobs AS duplicate
    JOIN jobs AS canonical
      ON canonical.job_type = 'compile_rules'
     AND canonical.project = duplicate.project
     AND canonical.state = duplicate.state
    JOIN _v069_survivors AS survivor
      ON survivor.id = canonical.id
     AND survivor.identity_kind = 'compile_rules'
    WHERE duplicate.job_type = 'compile_rules'
      AND duplicate.state IN ('pending', 'processing')
      AND duplicate.id <> canonical.id
);

UPDATE jobs
SET state = 'failed',
    lease_owner = NULL,
    lease_expires_epoch = NULL,
    next_retry_epoch = 0,
    last_error = CASE
        WHEN COALESCE(last_error, '') = '' THEN (
            SELECT marker FROM _v069_reconciled WHERE duplicate_id = jobs.id
        )
        ELSE
            substr(
                last_error,
                1,
                max(
                    0,
                    2000 - length((
                        SELECT marker FROM _v069_reconciled WHERE duplicate_id = jobs.id
                    )) - 1
                )
            ) || ' ' || (
                SELECT marker FROM _v069_reconciled WHERE duplicate_id = jobs.id
            )
    END,
    failure_class = 'permanent',
    failed_at_epoch = COALESCE(failed_at_epoch, (SELECT migration_now FROM _v069_context)),
    archived_at_epoch = NULL
WHERE id IN (SELECT duplicate_id FROM _v069_reconciled);

-- Fail the migration before any index is created if reconciliation left an
-- illegal group. CHECK constraints are valid outside triggers and abort the
-- enclosing migration transaction on any non-zero count.
CREATE TEMP TABLE _v069_validation (
    duplicate_group_count INTEGER NOT NULL CHECK (duplicate_group_count = 0)
);

INSERT INTO _v069_validation(duplicate_group_count)
SELECT COUNT(*)
FROM (
    SELECT host, job_type, project, COALESCE(session_id, '')
    FROM jobs
    WHERE job_type NOT IN ('dream', 'compile_rules')
      AND state IN ('pending', 'processing')
    GROUP BY host, job_type, project, COALESCE(session_id, '')
    HAVING COUNT(*) > 1
);

INSERT INTO _v069_validation(duplicate_group_count)
SELECT COUNT(*)
FROM (
    SELECT project
    FROM jobs
    WHERE job_type = 'dream'
      AND state IN ('pending', 'processing')
    GROUP BY project
    HAVING COUNT(*) > 1
);

INSERT INTO _v069_validation(duplicate_group_count)
SELECT COUNT(*)
FROM (
    SELECT project, state
    FROM jobs
    WHERE job_type = 'compile_rules'
      AND state IN ('pending', 'processing')
    GROUP BY project, state
    HAVING COUNT(*) > 1
);

CREATE UNIQUE INDEX idx_jobs_active_ordinary_unique
ON jobs(host, job_type, project, COALESCE(session_id, ''))
WHERE job_type NOT IN ('dream', 'compile_rules')
  AND state IN ('pending', 'processing');

CREATE UNIQUE INDEX idx_jobs_active_dream_unique
ON jobs(project)
WHERE job_type = 'dream'
  AND state IN ('pending', 'processing');

CREATE UNIQUE INDEX idx_jobs_active_compile_rules_unique
ON jobs(project, state)
WHERE job_type = 'compile_rules'
  AND state IN ('pending', 'processing');

DROP TABLE _v069_validation;
DROP TABLE _v069_dream_replay_result;
DROP TABLE _v069_dream_pending_order;
DROP TABLE _v069_survivors;
DROP TABLE _v069_context;
