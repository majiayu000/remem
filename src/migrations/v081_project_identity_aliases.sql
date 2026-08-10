-- v081_project_identity_aliases: canonical project routing without rewriting
-- historical capture paths.

CREATE TABLE project_identity_alias_events (
    id INTEGER PRIMARY KEY,
    alias_path TEXT NOT NULL,
    canonical_project_id INTEGER NOT NULL
        REFERENCES projects(id) ON DELETE RESTRICT,
    action TEXT NOT NULL
        CHECK(action IN ('activate', 'revoke')),
    proof_kind TEXT NOT NULL
        CHECK(proof_kind IN (
            'filesystem_canonicalization',
            'git_remote',
            'git_commit_membership'
        )),
    proof_payload_json TEXT NOT NULL
        CHECK(json_valid(proof_payload_json)),
    proof_sha256 TEXT NOT NULL
        CHECK(length(proof_sha256) = 64 AND proof_sha256 NOT GLOB '*[^0-9a-f]*'),
    source_inventory_sha256 TEXT NOT NULL
        CHECK(length(source_inventory_sha256) = 64
              AND source_inventory_sha256 NOT GLOB '*[^0-9a-f]*'),
    actor TEXT NOT NULL CHECK(trim(actor) <> ''),
    reason TEXT NOT NULL CHECK(trim(reason) <> ''),
    created_at_epoch INTEGER NOT NULL,
    CHECK(trim(alias_path) <> '')
);

CREATE INDEX idx_project_identity_alias_events_path
ON project_identity_alias_events(alias_path, created_at_epoch DESC, id DESC);

CREATE INDEX idx_project_identity_alias_events_target
ON project_identity_alias_events(canonical_project_id, created_at_epoch DESC, id DESC);

CREATE TABLE project_identity_aliases (
    alias_path TEXT PRIMARY KEY,
    canonical_project_id INTEGER NOT NULL
        REFERENCES projects(id) ON DELETE RESTRICT,
    status TEXT NOT NULL
        CHECK(status IN ('active', 'revoked')),
    last_event_id INTEGER NOT NULL UNIQUE
        REFERENCES project_identity_alias_events(id) ON DELETE RESTRICT,
    created_at_epoch INTEGER NOT NULL,
    updated_at_epoch INTEGER NOT NULL,
    CHECK(trim(alias_path) <> '')
);

CREATE INDEX idx_project_identity_aliases_target_status
ON project_identity_aliases(canonical_project_id, status, alias_path);
