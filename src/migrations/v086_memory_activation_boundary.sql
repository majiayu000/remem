-- v086_memory_activation_boundary: immutable, idempotent evidence for every
-- curated-memory activation routed through the GH969 boundary.

CREATE TABLE memory_activation_requests (
    activation_id TEXT PRIMARY KEY CHECK (
        typeof(activation_id) = 'text'
        AND length(trim(activation_id)) > 0
        AND instr(activation_id, char(0)) = 0
    ),
    request_sha256 TEXT NOT NULL CHECK (
        typeof(request_sha256) = 'text'
        AND length(request_sha256) = 64
        AND request_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    route_kind TEXT NOT NULL CHECK (route_kind IN (
        'rust_api', 'supplemental_save', 'candidate_promotion',
        'dream_consolidation', 'pack_import', 'scope_cleanup',
        'backup_import', 'web_restore', 'exact_recovery'
    )),
    actor_kind TEXT NOT NULL CHECK (actor_kind IN (
        'rust_api', 'agent', 'automatic_worker', 'operator', 'migration'
    )),
    source_operation TEXT NOT NULL CHECK (
        length(trim(source_operation)) > 0
        AND instr(source_operation, char(0)) = 0
    ),
    source_trust_class TEXT NOT NULL CHECK (source_trust_class IN (
        'external_content', 'pack', 'local_tool_output', 'repo_file', 'user_prompt'
    )),
    source_project TEXT NOT NULL CHECK (
        length(trim(source_project)) > 0 AND instr(source_project, char(0)) = 0
    ),
    project TEXT NOT NULL CHECK (
        length(trim(project)) > 0 AND instr(project, char(0)) = 0
    ),
    branch_present INTEGER NOT NULL CHECK (branch_present IN (0, 1)),
    branch TEXT,
    scope TEXT NOT NULL CHECK (scope IN ('project', 'global')),
    owner_scope TEXT NOT NULL CHECK (owner_scope IN (
        'repo', 'user', 'tool', 'domain', 'workstream', 'session', 'workspace'
    )),
    owner_key TEXT NOT NULL CHECK (
        length(trim(owner_key)) > 0 AND instr(owner_key, char(0)) = 0
    ),
    target_project TEXT,
    provenance_kind TEXT NOT NULL CHECK (provenance_kind IN (
        'supplemental_save', 'candidate', 'generated', 'pack',
        'backup', 'scope_plan', 'web_archive', 'exact_recovery', 'rust_api'
    )),
    provenance_ref TEXT NOT NULL CHECK (
        length(trim(provenance_ref)) > 0 AND instr(provenance_ref, char(0)) = 0
    ),
    payload_sha256 TEXT NOT NULL CHECK (
        length(payload_sha256) = 64
        AND payload_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    result_sha256 TEXT NOT NULL CHECK (
        length(result_sha256) = 64
        AND result_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    poisoning_verdict TEXT NOT NULL CHECK (poisoning_verdict IN (
        'clean', 'acknowledged', 'upstream_validated', 'exact_recovery'
    )),
    superseded_ids_json TEXT NOT NULL CHECK (
        typeof(superseded_ids_json) = 'text'
        AND json_valid(superseded_ids_json)
        AND json_type(superseded_ids_json) = 'array'
    ),
    result_memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE RESTRICT,
    claim_status TEXT CHECK (claim_status IN ('saved', 'disabled', 'failed')),
    claim_id INTEGER,
    claim_error TEXT,
    created_at_epoch INTEGER NOT NULL,
    CHECK (
        (branch_present = 0 AND branch IS NULL)
        OR (branch_present = 1 AND branch IS NOT NULL AND instr(branch, char(0)) = 0)
    ),
    CHECK (
        (scope = 'global' AND owner_scope = 'user' AND target_project IS NULL)
        OR scope = 'project'
    ),
    CHECK (
        (
            route_kind = 'supplemental_save'
            AND (
                (claim_status = 'saved' AND claim_id IS NOT NULL
                 AND claim_id > 0 AND claim_error IS NULL)
                OR (claim_status = 'disabled' AND claim_id IS NULL AND claim_error IS NULL)
                OR (
                    claim_status = 'failed' AND claim_id IS NULL
                    AND typeof(claim_error) = 'text'
                    AND length(trim(claim_error)) > 0
                    AND instr(claim_error, char(0)) = 0
                )
            )
        )
        OR (
            route_kind <> 'supplemental_save'
            AND claim_status IS NULL AND claim_id IS NULL AND claim_error IS NULL
        )
    )
);

CREATE INDEX idx_memory_activation_result
    ON memory_activation_requests(result_memory_id, created_at_epoch DESC);

CREATE TRIGGER memory_activation_requests_no_update
BEFORE UPDATE ON memory_activation_requests
BEGIN
    SELECT RAISE(ABORT, 'memory activation evidence is immutable');
END;

CREATE TRIGGER memory_activation_requests_no_delete
BEFORE DELETE ON memory_activation_requests
BEGIN
    SELECT RAISE(ABORT, 'memory activation evidence is immutable');
END;
