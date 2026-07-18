-- v071_raw_session_identity: durable transcript identity and lossless occurrences.

CREATE TABLE raw_session_identities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_root TEXT NOT NULL,
    transcript_path TEXT NOT NULL,
    fallback_session_id TEXT NOT NULL,
    canonical_session_id TEXT NOT NULL,
    project TEXT NOT NULL,
    legacy_project TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('active', 'conflict')),
    conflict_reason TEXT,
    contract_version INTEGER NOT NULL DEFAULT 0,
    observed_mtime_ns INTEGER NOT NULL,
    observed_size_bytes INTEGER NOT NULL,
    first_event_epoch INTEGER,
    last_event_epoch INTEGER,
    missing_event_time_count INTEGER NOT NULL DEFAULT 0,
    first_seen_at_epoch INTEGER NOT NULL,
    last_seen_at_epoch INTEGER NOT NULL,
    UNIQUE(source_root, transcript_path)
);

CREATE TABLE raw_session_identity_claims (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    transcript_identity_id INTEGER NOT NULL
        REFERENCES raw_session_identities(id) ON DELETE RESTRICT,
    claimed_session_id TEXT NOT NULL,
    identity_source TEXT NOT NULL
        CHECK(identity_source IN ('transcript_metadata', 'filename_fallback')),
    first_seen_at_epoch INTEGER NOT NULL,
    last_seen_at_epoch INTEGER NOT NULL,
    UNIQUE(transcript_identity_id, claimed_session_id, identity_source)
);

CREATE INDEX idx_raw_session_identities_fallback
    ON raw_session_identities(
        source_root, legacy_project, fallback_session_id, status
    );
CREATE INDEX idx_raw_session_identities_canonical
    ON raw_session_identities(
        source_root, project, canonical_session_id, status
    );
CREATE INDEX idx_raw_session_identity_claims_session
    ON raw_session_identity_claims(claimed_session_id, identity_source);

DROP TRIGGER IF EXISTS raw_messages_ai;
DROP TRIGGER IF EXISTS raw_messages_ad;
DROP TRIGGER IF EXISTS raw_messages_au;

ALTER TABLE raw_messages RENAME TO raw_messages_old;

CREATE TABLE raw_messages (
    id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL,
    project TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    source TEXT NOT NULL,
    branch TEXT,
    cwd TEXT,
    created_at_epoch INTEGER NOT NULL,
    source_root TEXT NOT NULL DEFAULT 'local',
    event_time_source TEXT NOT NULL DEFAULT 'legacy_unknown'
        CHECK(event_time_source IN (
            'transcript_event', 'ingest_fallback', 'legacy_unknown'
        )),
    transcript_identity_id INTEGER
        REFERENCES raw_session_identities(id) ON DELETE RESTRICT,
    transcript_record_ordinal INTEGER,
    CHECK(
        (transcript_identity_id IS NULL AND transcript_record_ordinal IS NULL)
        OR
        (transcript_identity_id IS NOT NULL AND transcript_record_ordinal IS NOT NULL)
    )
);

INSERT INTO raw_messages (
    id, session_id, project, role, content, content_hash, source, branch, cwd,
    created_at_epoch, source_root, event_time_source,
    transcript_identity_id, transcript_record_ordinal
)
SELECT
    id, session_id, project, role, content, content_hash, source, branch, cwd,
    created_at_epoch, source_root, 'legacy_unknown', NULL, NULL
FROM raw_messages_old;

DROP TABLE raw_messages_old;

CREATE UNIQUE INDEX idx_raw_messages_transcript_occurrence
    ON raw_messages(
        source_root, project, session_id,
        transcript_identity_id, transcript_record_ordinal
    )
    WHERE transcript_identity_id IS NOT NULL;
CREATE UNIQUE INDEX idx_raw_messages_non_transcript_content
    ON raw_messages(source_root, project, session_id, role, content_hash)
    WHERE transcript_identity_id IS NULL;
CREATE INDEX idx_raw_messages_project_created
    ON raw_messages(project, created_at_epoch DESC);
CREATE INDEX idx_raw_messages_session
    ON raw_messages(session_id, created_at_epoch);
CREATE INDEX idx_raw_messages_created_source_project_session
    ON raw_messages(created_at_epoch DESC, source_root, project, session_id);
CREATE INDEX idx_raw_messages_transcript_time
    ON raw_messages(transcript_identity_id, event_time_source, created_at_epoch);

CREATE TRIGGER raw_messages_ai AFTER INSERT ON raw_messages BEGIN
    INSERT INTO raw_messages_fts(rowid, content) VALUES (new.id, new.content);
END;
CREATE TRIGGER raw_messages_ad AFTER DELETE ON raw_messages BEGIN
    INSERT INTO raw_messages_fts(raw_messages_fts, rowid, content)
    VALUES ('delete', old.id, old.content);
END;
CREATE TRIGGER raw_messages_au AFTER UPDATE ON raw_messages BEGIN
    INSERT INTO raw_messages_fts(raw_messages_fts, rowid, content)
    VALUES ('delete', old.id, old.content);
    INSERT INTO raw_messages_fts(rowid, content) VALUES (new.id, new.content);
END;

INSERT INTO raw_messages_fts(raw_messages_fts) VALUES ('delete-all');
INSERT INTO raw_messages_fts(rowid, content)
SELECT id, content FROM raw_messages;
