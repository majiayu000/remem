use anyhow::{bail, Result};
use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::types::{BenchCondition, RunReport};
use crate::context_bundle::{ContextAudit, DegradedMode};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RememContextAuditStatus {
    Verified,
    ContractFailure,
    NotApplicable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RememContextAuditSnapshot {
    pub injection_run_id: String,
    pub bundle_schema_version: u32,
    pub plan_schema_version: u32,
    pub policy_version: String,
    pub relevance_policy_version: String,
    pub plan_hash: String,
    pub audit_hash: String,
    pub injection_binding_hash: String,
    pub degraded_mode: DegradedMode,
    pub candidates_considered: u32,
    pub selected_count: u32,
    pub dropped_count: u32,
    pub token_budget: u32,
    pub token_estimate: u32,
    pub truncation_reason: Option<String>,
    pub canonical_audit_json: String,
}

pub(crate) fn load_context_audit_snapshot(
    conn: &Connection,
    injection_run_id: &str,
) -> Result<Option<RememContextAuditSnapshot>> {
    let Some(persisted) = crate::context_bundle::persistence::load_verified_context_bundle_audit(
        conn,
        injection_run_id,
    )?
    else {
        return Ok(None);
    };
    let snapshot = snapshot_from_persisted(persisted);
    verify_context_audit_snapshot(&snapshot)?;
    Ok(Some(snapshot))
}

pub fn verify_context_audit_snapshot(snapshot: &RememContextAuditSnapshot) -> Result<()> {
    if snapshot.injection_run_id.trim().is_empty() {
        bail!("coding-bench ContextAudit injection_run_id must not be blank");
    }
    let (audit, actual_hash) =
        crate::context_bundle::persistence::decode_verified_context_audit_json(
            &snapshot.canonical_audit_json,
            snapshot.plan_schema_version,
        )?;
    if actual_hash != snapshot.audit_hash {
        bail!(
            "coding-bench ContextAudit hash mismatch for injection_run_id={}: stored={} actual={actual_hash}",
            snapshot.injection_run_id,
            snapshot.audit_hash
        );
    }
    let actual_binding =
        context_audit_binding_hash(&snapshot.injection_run_id, &snapshot.audit_hash);
    if actual_binding != snapshot.injection_binding_hash {
        bail!(
            "coding-bench ContextAudit injection binding mismatch for injection_run_id={}",
            snapshot.injection_run_id
        );
    }
    verify_summary(snapshot, &audit)
}

pub(crate) fn context_audit_binding_hash(injection_run_id: &str, audit_hash: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"remem-coding-bench-context-audit-binding-v1\0");
    hasher.update((injection_run_id.len() as u64).to_be_bytes());
    hasher.update(injection_run_id.as_bytes());
    hasher.update((audit_hash.len() as u64).to_be_bytes());
    hasher.update(audit_hash.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(crate) fn verify_snapshot_against_persisted_injection(
    conn: &Connection,
    snapshot: &RememContextAuditSnapshot,
) -> Result<()> {
    let actual =
        load_context_audit_snapshot(conn, &snapshot.injection_run_id)?.ok_or_else(|| {
            anyhow::anyhow!(
                "coding-bench ContextAudit missing for injection_run_id={}",
                snapshot.injection_run_id
            )
        })?;
    if actual != *snapshot {
        bail!(
            "coding-bench ContextAudit snapshot differs from persisted injection_run_id={}",
            snapshot.injection_run_id
        );
    }
    Ok(())
}

pub(crate) fn validate_run_context_audit(run: &RunReport) -> Result<()> {
    match run.condition {
        BenchCondition::Remem => match run.context_audit_status {
            RememContextAuditStatus::Verified => {
                if run.context_audit_failure_reason.is_some()
                    || run.runtime_contract_failure
                    || run.runtime_contract_failure_reason.is_some()
                {
                    bail!(
                        "remem coding-bench run {}#{} has verified ContextAudit with contract-failure state",
                        run.task_id,
                        run.run_index
                    );
                }
                let snapshot = run.remem_context_audit.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "remem coding-bench run {}#{} is missing ContextAudit snapshot",
                        run.task_id,
                        run.run_index
                    )
                })?;
                verify_context_audit_snapshot(snapshot)?;
            }
            RememContextAuditStatus::ContractFailure => {
                if run
                    .context_audit_failure_reason
                    .as_deref()
                    .is_none_or(|reason| reason.trim().is_empty())
                    || !run.runtime_contract_failure
                    || run
                        .runtime_contract_failure_reason
                        .as_deref()
                        .is_none_or(|reason| reason.trim().is_empty())
                    || run.remem_context_audit.is_some()
                {
                    bail!(
                        "remem coding-bench run {}#{} has inconsistent ContextAudit contract-failure state",
                        run.task_id,
                        run.run_index
                    );
                }
            }
            RememContextAuditStatus::NotApplicable => bail!(
                "remem coding-bench run {}#{} must not mark ContextAudit not_applicable",
                run.task_id,
                run.run_index
            ),
        },
        BenchCondition::NoMemory | BenchCondition::CuratedFile => {
            if run.context_audit_status != RememContextAuditStatus::NotApplicable
                || run.context_audit_failure_reason.is_some()
                || run.remem_context_audit.is_some()
                || run.runtime_contract_failure
                || run.runtime_contract_failure_reason.is_some()
            {
                bail!(
                    "{} coding-bench run {}#{} must mark ContextAudit not_applicable",
                    run.condition.as_str(),
                    run.task_id,
                    run.run_index
                );
            }
        }
    }
    Ok(())
}

fn snapshot_from_persisted(
    persisted: crate::context_bundle::persistence::PersistedContextBundleAudit,
) -> RememContextAuditSnapshot {
    let audit = persisted.audit;
    let injection_binding_hash =
        context_audit_binding_hash(&persisted.injection_run_id, &persisted.audit_hash);
    RememContextAuditSnapshot {
        injection_run_id: persisted.injection_run_id,
        bundle_schema_version: persisted.bundle_schema_version,
        plan_schema_version: persisted.plan_schema_version,
        policy_version: audit.policy_version,
        relevance_policy_version: audit.relevance_policy_version,
        plan_hash: audit.plan_hash,
        audit_hash: persisted.audit_hash,
        injection_binding_hash,
        degraded_mode: audit.degraded_mode,
        candidates_considered: audit.candidates_considered,
        selected_count: audit.selected_count,
        dropped_count: audit.dropped_count,
        token_budget: audit.token_budget,
        token_estimate: audit.token_estimate,
        truncation_reason: audit.truncation_reason,
        canonical_audit_json: persisted.canonical_audit_json,
    }
}

fn verify_summary(snapshot: &RememContextAuditSnapshot, audit: &ContextAudit) -> Result<()> {
    let matches = snapshot.bundle_schema_version == audit.schema_version
        && snapshot.policy_version == audit.policy_version
        && snapshot.relevance_policy_version == audit.relevance_policy_version
        && snapshot.plan_hash == audit.plan_hash
        && snapshot.degraded_mode == audit.degraded_mode
        && snapshot.candidates_considered == audit.candidates_considered
        && snapshot.selected_count == audit.selected_count
        && snapshot.dropped_count == audit.dropped_count
        && snapshot.token_budget == audit.token_budget
        && snapshot.token_estimate == audit.token_estimate
        && snapshot.truncation_reason == audit.truncation_reason;
    if !matches {
        bail!(
            "coding-bench ContextAudit summary mismatch for injection_run_id={}",
            snapshot.injection_run_id
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_bundle::{ContextAudit, CONTEXT_BUNDLE_SCHEMA_VERSION};

    fn snapshot() -> Result<RememContextAuditSnapshot> {
        let audit = ContextAudit {
            schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
            policy_version: "retrieval_router_v2".to_string(),
            relevance_policy_version: "sessionstart_significant_token_v1".to_string(),
            plan_hash: "a".repeat(64),
            degraded_mode: DegradedMode::Full,
            candidates_considered: 2,
            selected_count: 1,
            dropped_count: 1,
            token_estimate: 7,
            token_budget: 100,
            truncation_reason: Some("section_budget".to_string()),
            entries: Vec::new(),
        };
        let (canonical_audit_json, audit_hash) =
            crate::context_bundle::persistence::canonical_context_audit(
                &audit,
                crate::context_bundle::persistence::PERSISTED_PLAN_SCHEMA_V1,
            )?;
        let injection_run_id = "run-1".to_string();
        let injection_binding_hash = context_audit_binding_hash(&injection_run_id, &audit_hash);
        Ok(RememContextAuditSnapshot {
            injection_run_id,
            bundle_schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
            plan_schema_version: crate::context_bundle::persistence::PERSISTED_PLAN_SCHEMA_V1,
            policy_version: audit.policy_version,
            relevance_policy_version: audit.relevance_policy_version,
            plan_hash: audit.plan_hash,
            audit_hash,
            injection_binding_hash,
            degraded_mode: audit.degraded_mode,
            candidates_considered: audit.candidates_considered,
            selected_count: audit.selected_count,
            dropped_count: audit.dropped_count,
            token_budget: audit.token_budget,
            token_estimate: audit.token_estimate,
            truncation_reason: audit.truncation_reason,
            canonical_audit_json,
        })
    }

    #[test]
    fn verifier_recomputes_hash_and_summary() -> Result<()> {
        let original = snapshot()?;
        verify_context_audit_snapshot(&original)?;

        let mut wrong_hash = original.clone();
        wrong_hash.audit_hash = "b".repeat(64);
        assert!(verify_context_audit_snapshot(&wrong_hash)
            .unwrap_err()
            .to_string()
            .contains("hash mismatch"));

        let mut wrong_run_id = snapshot()?;
        wrong_run_id.injection_run_id = "run-2".to_string();
        assert!(verify_context_audit_snapshot(&wrong_run_id)
            .unwrap_err()
            .to_string()
            .contains("injection binding mismatch"));

        let mut blank_run_id = snapshot()?;
        blank_run_id.injection_run_id = "  ".to_string();
        assert!(verify_context_audit_snapshot(&blank_run_id)
            .unwrap_err()
            .to_string()
            .contains("must not be blank"));

        let mut wrong_summary = original;
        wrong_summary.selected_count = 2;
        assert!(verify_context_audit_snapshot(&wrong_summary)
            .unwrap_err()
            .to_string()
            .contains("summary mismatch"));

        let mut unsupported_version = snapshot()?;
        unsupported_version.plan_schema_version = 2;
        assert!(verify_context_audit_snapshot(&unsupported_version)
            .unwrap_err()
            .to_string()
            .contains("unsupported persisted retrieval plan schema version 2"));
        Ok(())
    }
}
