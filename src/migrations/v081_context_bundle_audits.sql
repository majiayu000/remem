-- v081_context_bundle_audits: append-only, payload-free Context Bundle audit
-- history for production SessionStart emissions (GH-932).

CREATE TABLE context_bundle_audits (
    id INTEGER PRIMARY KEY,
    injection_run_id TEXT NOT NULL UNIQUE,
    bundle_schema_version INTEGER NOT NULL CHECK (bundle_schema_version > 0),
    plan_schema_version INTEGER NOT NULL CHECK (plan_schema_version > 0),
    policy_version TEXT NOT NULL,
    relevance_policy_version TEXT NOT NULL,
    plan_hash TEXT NOT NULL CHECK (length(plan_hash) = 64),
    audit_hash TEXT NOT NULL CHECK (length(audit_hash) = 64),
    degraded_mode TEXT NOT NULL
        CHECK (degraded_mode IN ('full', 'canonical_only', 'blocked')),
    candidates_considered INTEGER NOT NULL CHECK (candidates_considered >= 0),
    selected_count INTEGER NOT NULL CHECK (selected_count >= 0),
    dropped_count INTEGER NOT NULL CHECK (dropped_count >= 0),
    token_budget INTEGER NOT NULL CHECK (token_budget > 0),
    token_estimate INTEGER NOT NULL CHECK (token_estimate >= 0),
    truncation_reason TEXT,
    audit_json TEXT NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    CHECK (selected_count + dropped_count = candidates_considered)
);

CREATE INDEX idx_context_bundle_audits_created
    ON context_bundle_audits(created_at_epoch);

CREATE INDEX idx_context_bundle_audits_plan
    ON context_bundle_audits(plan_hash, created_at_epoch DESC);

CREATE TRIGGER context_bundle_audits_require_items
BEFORE INSERT ON context_bundle_audits
WHEN NOT EXISTS (
    SELECT 1 FROM context_injection_items
    WHERE injection_run_id = NEW.injection_run_id
)
BEGIN
    SELECT RAISE(ABORT, 'context bundle audit requires context injection items');
END;

CREATE TRIGGER context_bundle_audits_no_duplicate_insert
BEFORE INSERT ON context_bundle_audits
WHEN EXISTS (
    SELECT 1 FROM context_bundle_audits
    WHERE injection_run_id = NEW.injection_run_id
)
BEGIN
    SELECT RAISE(ABORT, 'context bundle audit is append-only');
END;

CREATE TRIGGER context_bundle_audits_immutable_update
BEFORE UPDATE ON context_bundle_audits
BEGIN
    SELECT RAISE(ABORT, 'context bundle audit is immutable');
END;
