-- v091_raw_session_host: authoritative coding-agent host on transcript identity.

ALTER TABLE raw_session_identities ADD COLUMN host TEXT
    CHECK(host IS NULL OR host IN ('claude-code', 'codex-cli', 'cursor'));

ALTER TABLE raw_session_identities ADD COLUMN session_mode TEXT NOT NULL DEFAULT 'unknown'
    CHECK(session_mode IN ('interactive', 'unattended', 'subagent', 'unknown'));

UPDATE raw_session_identities
SET host = CASE
    WHEN instr('/' || replace(transcript_path, char(92), '/'), '/.claude/projects/') > 0
         AND instr('/' || replace(transcript_path, char(92), '/'), '/.codex/sessions/') = 0
         AND instr('/' || replace(transcript_path, char(92), '/'), '/.cursor/') = 0 THEN 'claude-code'
    WHEN instr('/' || replace(transcript_path, char(92), '/'), '/.codex/sessions/') > 0
         AND instr('/' || replace(transcript_path, char(92), '/'), '/.claude/projects/') = 0
         AND instr('/' || replace(transcript_path, char(92), '/'), '/.cursor/') = 0 THEN 'codex-cli'
    WHEN instr('/' || replace(transcript_path, char(92), '/'), '/.cursor/') > 0
         AND instr('/' || replace(transcript_path, char(92), '/'), '/.claude/projects/') = 0
         AND instr('/' || replace(transcript_path, char(92), '/'), '/.codex/sessions/') = 0 THEN 'cursor'
    ELSE NULL
END;

UPDATE raw_session_identities
SET status = 'active',
    conflict_reason = NULL,
    canonical_session_id = COALESCE(
        (
            SELECT MIN(c.claimed_session_id)
            FROM raw_session_identities AS grouped
            JOIN raw_session_identity_claims AS c
              ON c.transcript_identity_id = grouped.id
            WHERE grouped.host = raw_session_identities.host
              AND grouped.source_root = raw_session_identities.source_root
              AND grouped.fallback_session_id = raw_session_identities.fallback_session_id
              AND c.identity_source = 'transcript_metadata'
        ),
        fallback_session_id
    )
WHERE host IS NOT NULL
  AND status = 'conflict'
  AND conflict_reason = 'conflicting_metadata_claims'
  AND (
      SELECT COUNT(DISTINCT c.claimed_session_id)
      FROM raw_session_identities AS grouped
      JOIN raw_session_identity_claims AS c
        ON c.transcript_identity_id = grouped.id
      WHERE grouped.host = raw_session_identities.host
        AND grouped.source_root = raw_session_identities.source_root
        AND grouped.fallback_session_id = raw_session_identities.fallback_session_id
        AND c.identity_source = 'transcript_metadata'
  ) <= 1;

CREATE INDEX idx_raw_session_identities_host_canonical
    ON raw_session_identities(
        host, source_root, project, canonical_session_id, status
    );
