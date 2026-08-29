-- v091_raw_session_host: authoritative coding-agent host on transcript identity.

ALTER TABLE raw_session_identities ADD COLUMN host TEXT
    CHECK(host IS NULL OR host IN ('claude-code', 'codex-cli', 'cursor'));

UPDATE raw_session_identities
SET host = CASE
    WHEN instr('/' || transcript_path, '/.claude/projects/') > 0
         AND instr('/' || transcript_path, '/.codex/sessions/') = 0
         AND instr('/' || transcript_path, '/.cursor/') = 0 THEN 'claude-code'
    WHEN instr('/' || transcript_path, '/.codex/sessions/') > 0
         AND instr('/' || transcript_path, '/.claude/projects/') = 0
         AND instr('/' || transcript_path, '/.cursor/') = 0 THEN 'codex-cli'
    WHEN instr('/' || transcript_path, '/.cursor/') > 0
         AND instr('/' || transcript_path, '/.claude/projects/') = 0
         AND instr('/' || transcript_path, '/.codex/sessions/') = 0 THEN 'cursor'
    ELSE NULL
END;

CREATE INDEX idx_raw_session_identities_host_canonical
    ON raw_session_identities(
        host, source_root, project, canonical_session_id, status
    );
