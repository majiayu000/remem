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

CREATE INDEX idx_raw_session_identities_host_canonical
    ON raw_session_identities(
        host, source_root, project, canonical_session_id, status
    );
