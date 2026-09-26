-- Version zero preserves legacy provenance until the native prefix verifies it.
ALTER TABLE raw_session_identities
    ADD COLUMN session_mode_version INTEGER NOT NULL DEFAULT 0
    CHECK (session_mode_version IN (0, 1));
