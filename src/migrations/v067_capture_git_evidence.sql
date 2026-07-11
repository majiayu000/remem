-- v067_capture_git_evidence: persist explicit, capture-time commit evidence
-- separately from event content and key links by the durable capture session.

CREATE TABLE captured_event_commits (
    event_row_id INTEGER NOT NULL REFERENCES captured_events(id) ON DELETE CASCADE,
    sha TEXT NOT NULL,
    metadata_json TEXT NOT NULL,
    evidence_kind TEXT NOT NULL,
    evidence_locator TEXT,
    PRIMARY KEY(event_row_id, sha, evidence_kind)
);

CREATE INDEX idx_captured_event_commits_event
    ON captured_event_commits(event_row_id, evidence_kind, sha);

-- v018 keyed links only by the externally supplied session string. Rebuild
-- the table so two hosts that reuse the same raw session ID cannot overwrite
-- one another. Historical rows remain queryable with a NULL session_row_id.
ALTER TABLE git_commit_sessions RENAME TO git_commit_sessions_v018;

CREATE TABLE git_commit_sessions (
    id INTEGER PRIMARY KEY,
    commit_id INTEGER NOT NULL REFERENCES git_commits(id) ON DELETE CASCADE,
    session_row_id INTEGER REFERENCES sessions(id) ON DELETE RESTRICT,
    session_id TEXT NOT NULL,
    memory_session_id TEXT,
    source TEXT NOT NULL,
    linked_at_epoch INTEGER NOT NULL
);

INSERT INTO git_commit_sessions
    (commit_id, session_row_id, session_id, memory_session_id, source, linked_at_epoch)
SELECT commit_id, NULL, session_id, memory_session_id, source, linked_at_epoch
FROM git_commit_sessions_v018;

DROP TABLE git_commit_sessions_v018;

-- Only a capture-rollup identity can prove which durable capture session an
-- old string-keyed link belonged to. Ambiguous legacy identities stay NULL.
WITH mapped AS (
    SELECT links.id AS link_id, sessions.id AS session_row_id
    FROM git_commit_sessions links
    JOIN git_commits commits ON commits.id = links.commit_id
    JOIN sessions
      ON links.memory_session_id = 'capture-rollup-' || sessions.id
     AND links.session_id = sessions.session_id
    JOIN projects
      ON projects.id = sessions.project_id
     AND projects.project_path = commits.project
    WHERE links.session_row_id IS NULL
)
UPDATE git_commit_sessions
SET session_row_id = (
    SELECT mapped.session_row_id
    FROM mapped
    WHERE mapped.link_id = git_commit_sessions.id
)
WHERE id IN (SELECT link_id FROM mapped);

CREATE UNIQUE INDEX idx_git_commit_sessions_commit_session_row
    ON git_commit_sessions(commit_id, session_row_id)
    WHERE session_row_id IS NOT NULL;
CREATE UNIQUE INDEX idx_git_commit_sessions_commit_legacy_session
    ON git_commit_sessions(commit_id, session_id)
    WHERE session_row_id IS NULL;
CREATE INDEX idx_git_commit_sessions_session
    ON git_commit_sessions(session_id);
CREATE INDEX idx_git_commit_sessions_memory_session
    ON git_commit_sessions(memory_session_id);
CREATE INDEX idx_git_commit_sessions_session_row
    ON git_commit_sessions(session_row_id)
    WHERE session_row_id IS NOT NULL;
