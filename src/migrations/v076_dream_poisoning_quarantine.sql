-- v076_dream_poisoning_quarantine: bind blocked Dream model output to the
-- exact source-memory cluster and review candidate without exposing it as an
-- active memory.

CREATE TABLE dream_quarantine_artifacts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    version INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1),
    project TEXT NOT NULL,
    cluster_signature TEXT NOT NULL,
    member_ids_json TEXT NOT NULL CHECK (
        typeof(member_ids_json) = 'text'
        AND json_valid(member_ids_json)
        AND json_type(member_ids_json) = 'array'
        AND json_array_length(member_ids_json) > 0
    ),
    source_candidate_id INTEGER NOT NULL
        REFERENCES memory_candidates(id) ON DELETE RESTRICT,
    decision_kind TEXT NOT NULL CHECK (
        decision_kind IN ('merge', 'no_merge', 'conflict')
    ),
    decision_ids_json TEXT NOT NULL CHECK (
        typeof(decision_ids_json) = 'text'
        AND json_valid(decision_ids_json)
        AND json_type(decision_ids_json) = 'array'
    ),
    decision_payload_sha256 TEXT NOT NULL CHECK (
        typeof(decision_payload_sha256) = 'text'
        AND length(decision_payload_sha256) = 64
        AND decision_payload_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    intended_superseded_ids_json TEXT NOT NULL CHECK (
        typeof(intended_superseded_ids_json) = 'text'
        AND json_valid(intended_superseded_ids_json)
        AND json_type(intended_superseded_ids_json) = 'array'
    ),
    generated_topic_key TEXT,
    generated_memory_type TEXT,
    generated_title TEXT,
    generated_content TEXT,
    generated_field TEXT NOT NULL CHECK (
        (decision_kind = 'merge' AND generated_field IN (
            'dream.topic_key', 'dream.memory_type', 'dream.title',
            'dream.content', 'dream.title_content'
        ))
        OR (decision_kind = 'no_merge' AND generated_field = 'dream.no_merge_reason')
        OR (decision_kind = 'conflict' AND generated_field = 'dream.conflict_reason')
    ),
    pattern_id TEXT NOT NULL CHECK (length(trim(pattern_id)) > 0),
    pattern_version INTEGER NOT NULL CHECK (pattern_version > 0),
    source_operation TEXT NOT NULL CHECK (source_operation = 'dream'),
    source_trust_class TEXT NOT NULL
        CHECK (source_trust_class = 'external_content'),
    occurrence_count INTEGER NOT NULL DEFAULT 1 CHECK (occurrence_count >= 1),
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    CHECK (updated_at_epoch >= created_at_epoch),
    CHECK (version = occurrence_count),
    CHECK (
        (
            decision_kind = 'merge'
            AND generated_topic_key IS NOT NULL
            AND length(trim(generated_topic_key)) > 0
            AND generated_memory_type IS NOT NULL
            AND length(trim(generated_memory_type)) > 0
            AND generated_title IS NOT NULL
            AND length(trim(generated_title)) > 0
            AND generated_content IS NOT NULL
            AND length(trim(generated_content)) > 0
        )
        OR
        (
            decision_kind != 'merge'
            AND generated_topic_key IS NULL
            AND generated_memory_type IS NULL
            AND generated_title IS NULL
            AND generated_content IS NULL
        )
    ),
    UNIQUE(project, cluster_signature, source_candidate_id)
);

CREATE TRIGGER dream_quarantine_artifacts_no_replace
BEFORE INSERT ON dream_quarantine_artifacts
WHEN EXISTS (
    SELECT 1 FROM dream_quarantine_artifacts
    WHERE id = NEW.id
       OR (
           project = NEW.project
           AND cluster_signature = NEW.cluster_signature
           AND source_candidate_id = NEW.source_candidate_id
       )
)
BEGIN
    SELECT RAISE(ABORT, 'Dream quarantine artifact already exists');
END;

CREATE TRIGGER dream_quarantine_artifacts_initial_counters
BEFORE INSERT ON dream_quarantine_artifacts
WHEN NEW.version != 1 OR NEW.occurrence_count != 1
BEGIN
    SELECT RAISE(ABORT, 'Dream quarantine artifact counters must start at one');
END;

CREATE TRIGGER dream_quarantine_artifacts_validate_intended_insert
BEFORE INSERT ON dream_quarantine_artifacts
WHEN EXISTS (
        SELECT 1 FROM json_each(NEW.member_ids_json)
        WHERE type != 'integer' OR atom <= 0
    )
    OR EXISTS (
        SELECT 1
        FROM json_each(NEW.member_ids_json) AS earlier
        JOIN json_each(NEW.member_ids_json) AS later
          ON CAST(earlier.key AS INTEGER) < CAST(later.key AS INTEGER)
        WHERE earlier.atom >= later.atom
    )
    OR EXISTS (
        SELECT 1 FROM json_each(NEW.intended_superseded_ids_json)
        WHERE type != 'integer' OR atom <= 0
    )
    OR EXISTS (
        SELECT 1
        FROM json_each(NEW.intended_superseded_ids_json) AS earlier
        JOIN json_each(NEW.intended_superseded_ids_json) AS later
          ON CAST(earlier.key AS INTEGER) < CAST(later.key AS INTEGER)
        WHERE earlier.atom >= later.atom
    )
    OR EXISTS (
        SELECT 1 FROM json_each(NEW.decision_ids_json)
        WHERE type != 'integer' OR atom <= 0
    )
    OR EXISTS (
        SELECT 1
        FROM json_each(NEW.decision_ids_json) AS earlier
        JOIN json_each(NEW.decision_ids_json) AS later
          ON CAST(earlier.key AS INTEGER) < CAST(later.key AS INTEGER)
        WHERE earlier.atom >= later.atom
    )
    OR (
        NEW.decision_kind = 'merge'
        AND json_array_length(NEW.intended_superseded_ids_json) = 0
    )
    OR (
        NEW.decision_kind != 'merge'
        AND json_array_length(NEW.intended_superseded_ids_json) != 0
    )
    OR (
        NEW.decision_kind = 'merge'
        AND EXISTS (
            SELECT 1
            FROM json_each(NEW.intended_superseded_ids_json) AS intended
            WHERE NOT EXISTS (
                SELECT 1 FROM json_each(NEW.member_ids_json) AS member
                WHERE member.type = 'integer' AND member.atom = intended.atom
            )
        )
    )
    OR (
        NEW.decision_kind = 'merge'
        AND json(NEW.decision_ids_json)
            != json(NEW.intended_superseded_ids_json)
    )
    OR (
        NEW.decision_kind = 'conflict'
        AND json_array_length(NEW.decision_ids_json) < 2
    )
    OR (
        NEW.decision_kind = 'conflict'
        AND EXISTS (
            SELECT 1
            FROM json_each(NEW.decision_ids_json) AS decision_id
            WHERE NOT EXISTS (
                SELECT 1 FROM json_each(NEW.member_ids_json) AS member
                WHERE member.type = 'integer' AND member.atom = decision_id.atom
            )
        )
    )
    OR (
        NEW.decision_kind = 'no_merge'
        AND json_array_length(NEW.decision_ids_json) != 0
    )
BEGIN
    SELECT RAISE(ABORT, 'invalid Dream decision provenance');
END;

CREATE TRIGGER dream_quarantine_artifacts_validate_intended_update
BEFORE UPDATE OF decision_kind, decision_ids_json,
                 intended_superseded_ids_json, member_ids_json
ON dream_quarantine_artifacts
WHEN EXISTS (
        SELECT 1 FROM json_each(NEW.member_ids_json)
        WHERE type != 'integer' OR atom <= 0
    )
    OR EXISTS (
        SELECT 1
        FROM json_each(NEW.member_ids_json) AS earlier
        JOIN json_each(NEW.member_ids_json) AS later
          ON CAST(earlier.key AS INTEGER) < CAST(later.key AS INTEGER)
        WHERE earlier.atom >= later.atom
    )
    OR EXISTS (
        SELECT 1 FROM json_each(NEW.intended_superseded_ids_json)
        WHERE type != 'integer' OR atom <= 0
    )
    OR EXISTS (
        SELECT 1
        FROM json_each(NEW.intended_superseded_ids_json) AS earlier
        JOIN json_each(NEW.intended_superseded_ids_json) AS later
          ON CAST(earlier.key AS INTEGER) < CAST(later.key AS INTEGER)
        WHERE earlier.atom >= later.atom
    )
    OR EXISTS (
        SELECT 1 FROM json_each(NEW.decision_ids_json)
        WHERE type != 'integer' OR atom <= 0
    )
    OR EXISTS (
        SELECT 1
        FROM json_each(NEW.decision_ids_json) AS earlier
        JOIN json_each(NEW.decision_ids_json) AS later
          ON CAST(earlier.key AS INTEGER) < CAST(later.key AS INTEGER)
        WHERE earlier.atom >= later.atom
    )
    OR (
        NEW.decision_kind = 'merge'
        AND json_array_length(NEW.intended_superseded_ids_json) = 0
    )
    OR (
        NEW.decision_kind != 'merge'
        AND json_array_length(NEW.intended_superseded_ids_json) != 0
    )
    OR (
        NEW.decision_kind = 'merge'
        AND EXISTS (
            SELECT 1
            FROM json_each(NEW.intended_superseded_ids_json) AS intended
            WHERE NOT EXISTS (
                SELECT 1 FROM json_each(NEW.member_ids_json) AS member
                WHERE member.type = 'integer' AND member.atom = intended.atom
            )
        )
    )
    OR (
        NEW.decision_kind = 'merge'
        AND json(NEW.decision_ids_json)
            != json(NEW.intended_superseded_ids_json)
    )
    OR (
        NEW.decision_kind = 'conflict'
        AND json_array_length(NEW.decision_ids_json) < 2
    )
    OR (
        NEW.decision_kind = 'conflict'
        AND EXISTS (
            SELECT 1
            FROM json_each(NEW.decision_ids_json) AS decision_id
            WHERE NOT EXISTS (
                SELECT 1 FROM json_each(NEW.member_ids_json) AS member
                WHERE member.type = 'integer' AND member.atom = decision_id.atom
            )
        )
    )
    OR (
        NEW.decision_kind = 'no_merge'
        AND json_array_length(NEW.decision_ids_json) != 0
    )
BEGIN
    SELECT RAISE(ABORT, 'invalid Dream decision provenance');
END;

CREATE TRIGGER dream_quarantine_artifacts_immutable_payload
BEFORE UPDATE OF id, project, cluster_signature, member_ids_json,
                 source_candidate_id, decision_kind, decision_ids_json,
                 decision_payload_sha256, intended_superseded_ids_json,
                 generated_topic_key, generated_memory_type,
                 generated_title, generated_content, generated_field, pattern_id,
                 pattern_version, source_operation, source_trust_class,
                 created_at_epoch
ON dream_quarantine_artifacts
BEGIN
    SELECT RAISE(ABORT, 'Dream quarantine artifact payload is immutable');
END;

CREATE TRIGGER dream_quarantine_artifacts_monotonic_recurrence
BEFORE UPDATE OF version, occurrence_count, updated_at_epoch
ON dream_quarantine_artifacts
WHEN NEW.version != OLD.version + 1
    OR NEW.occurrence_count != OLD.occurrence_count + 1
    OR NEW.updated_at_epoch < OLD.updated_at_epoch
BEGIN
    SELECT RAISE(ABORT, 'Dream quarantine artifact recurrence is not monotonic');
END;

CREATE TRIGGER dream_quarantine_artifacts_no_delete
BEFORE DELETE ON dream_quarantine_artifacts
BEGIN
    SELECT RAISE(ABORT, 'Dream quarantine artifacts cannot be deleted');
END;

CREATE INDEX idx_dream_quarantine_project_recent
ON dream_quarantine_artifacts(project, updated_at_epoch DESC, id DESC);

CREATE INDEX idx_dream_quarantine_candidate
ON dream_quarantine_artifacts(source_candidate_id);

-- Immutable, route-bound identity ledger for externally sourced candidates.
-- Store only the content digest here: raw external content remains confined to
-- memory_candidates and cannot leak into the identity/audit surface.
CREATE TABLE external_candidate_identities (
    identity_sha256 TEXT PRIMARY KEY NOT NULL CHECK (
        typeof(identity_sha256) = 'text'
        AND length(identity_sha256) = 64
        AND identity_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    candidate_id INTEGER NOT NULL
        REFERENCES memory_candidates(id) ON DELETE RESTRICT,
    source_kind TEXT NOT NULL,
    memory_type TEXT NOT NULL CHECK (
        typeof(memory_type) = 'text' AND length(trim(memory_type)) > 0
    ),
    semantic_discriminator_sha256 TEXT CHECK (
        semantic_discriminator_sha256 IS NULL
        OR (
            typeof(semantic_discriminator_sha256) = 'text'
            AND length(semantic_discriminator_sha256) = 64
            AND semantic_discriminator_sha256 NOT GLOB '*[^0-9a-f]*'
        )
    ),
    source_project TEXT NOT NULL,
    owner_scope TEXT NOT NULL,
    owner_key TEXT NOT NULL,
    target_project TEXT,
    topic_key TEXT NOT NULL,
    text_sha256 TEXT NOT NULL CHECK (
        typeof(text_sha256) = 'text'
        AND length(text_sha256) = 64
        AND text_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    first_seen_epoch INTEGER NOT NULL,
    last_seen_epoch INTEGER NOT NULL,
    occurrence_count INTEGER NOT NULL CHECK (occurrence_count >= 1),
    CHECK (last_seen_epoch >= first_seen_epoch)
);

CREATE INDEX idx_external_candidate_identities_candidate
ON external_candidate_identities(candidate_id);

CREATE TRIGGER external_candidate_identities_no_replace
BEFORE INSERT ON external_candidate_identities
WHEN EXISTS (
    SELECT 1 FROM external_candidate_identities
    WHERE identity_sha256 = NEW.identity_sha256
)
BEGIN
    SELECT RAISE(ABORT, 'external candidate identity already exists');
END;

CREATE TRIGGER external_candidate_identities_immutable_update
BEFORE UPDATE OF identity_sha256, candidate_id, source_kind, memory_type,
                 semantic_discriminator_sha256, source_project, owner_scope, owner_key,
                 target_project, topic_key, text_sha256, first_seen_epoch
ON external_candidate_identities
BEGIN
    SELECT RAISE(ABORT, 'external candidate identity fields are immutable');
END;

CREATE TRIGGER external_candidate_identities_monotonic_recurrence
BEFORE UPDATE OF last_seen_epoch, occurrence_count
ON external_candidate_identities
WHEN NEW.last_seen_epoch < OLD.last_seen_epoch
  OR NEW.occurrence_count != OLD.occurrence_count + 1
BEGIN
    SELECT RAISE(ABORT, 'external candidate identity recurrence is not monotonic');
END;

CREATE TRIGGER external_candidate_identities_no_delete
BEFORE DELETE ON external_candidate_identities
BEGIN
    SELECT RAISE(ABORT, 'external candidate identity ledger entries cannot be deleted');
END;

CREATE TABLE external_candidate_recurrences (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    identity_sha256 TEXT NOT NULL
        REFERENCES external_candidate_identities(identity_sha256)
        ON DELETE RESTRICT,
    canonical_candidate_id INTEGER NOT NULL
        REFERENCES memory_candidates(id) ON DELETE RESTRICT,
    candidate_id INTEGER NOT NULL
        REFERENCES memory_candidates(id) ON DELETE RESTRICT,
    recurrence_kind TEXT NOT NULL CHECK (
        recurrence_kind IN (
            'review_candidate',
            'discarded_pattern',
            'acknowledged_pattern',
            'terminal_duplicate'
        )
    ),
    pattern_id TEXT,
    pattern_version INTEGER,
    occurred_at_epoch INTEGER NOT NULL,
    CHECK (
        (recurrence_kind = 'terminal_duplicate'
            AND pattern_id IS NULL AND pattern_version IS NULL)
        OR
        (recurrence_kind != 'terminal_duplicate'
            AND pattern_id IS NOT NULL AND pattern_version IS NOT NULL
            AND length(trim(pattern_id)) > 0 AND pattern_version > 0)
    )
);

CREATE INDEX idx_external_candidate_recurrences_identity_recent
ON external_candidate_recurrences(identity_sha256, id DESC);

CREATE INDEX idx_external_candidate_recurrences_candidate
ON external_candidate_recurrences(candidate_id);

CREATE TRIGGER external_candidate_recurrences_validate_insert
BEFORE INSERT ON external_candidate_recurrences
WHEN NOT EXISTS (
    SELECT 1 FROM external_candidate_identities
    WHERE identity_sha256 = NEW.identity_sha256
      AND candidate_id = NEW.canonical_candidate_id
)
BEGIN
    SELECT RAISE(ABORT, 'external recurrence canonical identity mismatch');
END;

CREATE TRIGGER external_candidate_recurrences_immutable_update
BEFORE UPDATE ON external_candidate_recurrences
BEGIN
    SELECT RAISE(ABORT, 'external candidate recurrence is immutable');
END;

CREATE TRIGGER external_candidate_recurrences_no_delete
BEFORE DELETE ON external_candidate_recurrences
BEGIN
    SELECT RAISE(ABORT, 'external candidate recurrences cannot be deleted');
END;
