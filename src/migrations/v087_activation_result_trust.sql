-- v087_activation_result_trust: persist the trust class of each activation
-- result without rewriting the already-applied v086 migration.

DROP TRIGGER memory_activation_requests_no_update;
DROP TRIGGER memory_activation_requests_no_delete;

ALTER TABLE memory_activation_requests RENAME TO memory_activation_requests_v086;

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
    result_source_trust_class TEXT NOT NULL CHECK (
        result_source_trust_class IN (
            'external_content', 'pack', 'local_tool_output', 'repo_file', 'user_prompt',
            'legacy_observed_external_content', 'legacy_observed_pack',
            'legacy_observed_local_tool_output', 'legacy_observed_repo_file',
            'legacy_observed_user_prompt'
        )
    ),
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

INSERT INTO memory_activation_requests (
    rowid, activation_id, request_sha256, route_kind, actor_kind, source_operation,
    source_trust_class, result_source_trust_class, source_project, project,
    branch_present, branch, scope, owner_scope, owner_key, target_project,
    provenance_kind, provenance_ref, payload_sha256, result_sha256,
    poisoning_verdict, superseded_ids_json, result_memory_id, claim_status,
    claim_id, claim_error, created_at_epoch
)
SELECT
    receipt.rowid, receipt.activation_id, receipt.request_sha256, receipt.route_kind,
    receipt.actor_kind, receipt.source_operation, receipt.source_trust_class,
    'legacy_observed_' || memory.source_trust_class,
    receipt.source_project, receipt.project,
    receipt.branch_present, receipt.branch, receipt.scope, receipt.owner_scope,
    receipt.owner_key, receipt.target_project, receipt.provenance_kind,
    receipt.provenance_ref, receipt.payload_sha256, receipt.result_sha256,
    receipt.poisoning_verdict, receipt.superseded_ids_json,
    receipt.result_memory_id, receipt.claim_status, receipt.claim_id,
    receipt.claim_error, receipt.created_at_epoch
FROM memory_activation_requests_v086 AS receipt
JOIN memories AS memory ON memory.id = receipt.result_memory_id;

DROP TABLE memory_activation_requests_v086;

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
