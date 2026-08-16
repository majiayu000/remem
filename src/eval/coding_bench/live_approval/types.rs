use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum RunPhase {
    Local,
    Smoke,
    Official,
}

impl RunPhase {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "local" => Some(Self::Local),
            "smoke" => Some(Self::Smoke),
            "official" => Some(Self::Official),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ApprovalTrustRoot {
    pub(super) schema_version: u32,
    pub(super) approval_key: VerificationKey,
    pub(super) supervisor_key: VerificationKey,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VerificationKey {
    pub(super) key_id: String,
    pub(super) algorithm: String,
    pub(super) public_key_base64: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DetachedSignature {
    pub(super) key_id: String,
    pub(super) algorithm: String,
    pub(super) signature_base64: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SignedApprovalEnvelope {
    pub(super) schema_version: u32,
    pub(super) payload: LiveApprovalPayload,
    pub(super) signature: DetachedSignature,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LiveApprovalPayload {
    pub(super) approval_id: String,
    pub(super) repository: String,
    pub(super) default_branch: String,
    pub(super) approved_commit: String,
    pub(super) run_phase: RunPhase,
    pub(super) matrix_namespace: String,
    pub(super) plan_sha256: String,
    pub(super) fixture_sha256: String,
    pub(super) condition_registry_sha256: String,
    pub(super) remem_executable_sha256: String,
    pub(super) runner_executable_sha256: String,
    pub(super) memory_config_sha256: String,
    pub(super) curator_manifest_sha256: String,
    pub(super) runner: ApprovedRunner,
    pub(super) pricing: ApprovedPricing,
    pub(super) caps: ApprovedCaps,
    pub(super) supervisor: ApprovedSupervisor,
    pub(super) ledger: ApprovedLedger,
    pub(super) sigstore: ApprovedSigstore,
    pub(super) not_before_epoch: i64,
    pub(super) expires_at_epoch: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct ApprovedRunner {
    pub(super) runner: String,
    pub(super) model: String,
    pub(super) provider: String,
    pub(super) reasoning_effort: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ApprovedPricing {
    pub(super) currency: String,
    pub(super) input_usd_micros_per_million_tokens: u64,
    pub(super) output_usd_micros_per_million_tokens: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ApprovedCaps {
    pub(super) max_agent_calls: u64,
    pub(super) max_provider_calls: u64,
    pub(super) max_input_tokens: u64,
    pub(super) max_output_tokens: u64,
    pub(super) max_cost_usd_micros: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ApprovedSupervisor {
    pub(super) identity: String,
    pub(super) signing_key_id: String,
    pub(super) executable_sha256: String,
    pub(super) required_uid: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ApprovedLedger {
    pub(super) writer_app_id: u64,
    pub(super) writer_app_slug: String,
    pub(super) signing_key_id: String,
    pub(super) ledger_ref: String,
    pub(super) update_ruleset_id: u64,
    pub(super) update_ruleset_sha256: String,
    pub(super) no_bypass_ruleset_id: u64,
    pub(super) no_bypass_ruleset_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ApprovedSigstore {
    pub(super) trusted_root_sha256: String,
    pub(super) signing_config_sha256: String,
    pub(super) rekor_log_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SignedSupervisorAttestation {
    pub(super) schema_version: u32,
    pub(super) payload: SupervisorAttestationPayload,
    pub(super) signature: DetachedSignature,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SupervisorAttestationPayload {
    pub(super) approval_id: String,
    pub(super) plan_sha256: String,
    pub(super) supervisor_identity: String,
    pub(super) supervisor_executable_sha256: String,
    pub(super) supervisor_uid: u32,
    pub(super) platform: String,
    pub(super) nofollow_open: bool,
    pub(super) same_handle_execution: bool,
    pub(super) caller_cannot_access_signing_key: bool,
    pub(super) not_before_epoch: i64,
    pub(super) expires_at_epoch: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct CanonicalPlanBinding {
    pub(super) schema: String,
    pub(super) repository: String,
    pub(super) approved_commit: String,
    pub(super) run_phase: RunPhase,
    pub(super) matrix_namespace: String,
    pub(super) matrix: String,
    pub(super) task_set: String,
    pub(super) runs_per_condition: usize,
    pub(super) tuples: Vec<ApprovedTuple>,
    pub(super) fixture_sha256: String,
    pub(super) condition_registry_sha256: String,
    pub(super) remem_executable_sha256: String,
    pub(super) runner_executable_sha256: String,
    pub(super) memory_config_sha256: String,
    pub(super) curator_manifest_sha256: String,
    pub(super) runner: ApprovedRunner,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct ApprovedTuple {
    pub(super) condition: String,
    pub(super) task_id: String,
    pub(super) run_index: usize,
}

#[derive(Debug, Clone)]
pub(super) struct ExpectedLiveBinding {
    pub(super) phase: RunPhase,
    pub(super) namespace: String,
    pub(super) head_commit: String,
    pub(super) plan: CanonicalPlanBinding,
    pub(super) plan_sha256: String,
    pub(super) supervisor_executable_sha256: String,
    pub(super) supervisor_uid: u32,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct LiveApprovalReport {
    pub schema_version: u32,
    pub passed: bool,
    pub gate_scope: String,
    pub dispatch_authorized: bool,
    pub provider_or_agent_calls: u64,
    pub approval_id: String,
    pub run_phase: RunPhase,
    pub matrix_namespace: String,
    pub approved_commit: String,
    pub plan_sha256: String,
    pub approval_payload_sha256: String,
    pub supervisor_attestation_sha256: String,
    pub approval_expires_at_epoch: i64,
    pub supervisor_attestation_expires_at_epoch: i64,
    pub checks: Vec<String>,
}
