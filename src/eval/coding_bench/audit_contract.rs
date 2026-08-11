use anyhow::{bail, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use super::types::{BenchCondition, RunReport};
use crate::context_bundle::{ChannelKind, ContextAudit, DegradedMode};

const SUPPORTED_CONTEXT_BUNDLE_SCHEMA_V1: u32 = 1;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RememContextAuditStatus {
    Verified,
    ContractFailure,
    NotApplicable,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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
    injected_context: &str,
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
    let context_hashes = {
        let mut statement = conn.prepare(
            "SELECT DISTINCT context_hash
             FROM context_injection_items
             WHERE injection_run_id = ?1",
        )?;
        let hashes = statement
            .query_map([&snapshot.injection_run_id], |row| {
                row.get::<_, Option<String>>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        hashes
    };
    if context_hashes.len() != 1 {
        bail!(
            "coding-bench ContextAudit injection_run_id={} must link exactly one emitted context hash",
            snapshot.injection_run_id
        );
    }
    let persisted_context_hash = context_hashes[0].as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "coding-bench ContextAudit injection_run_id={} has no emitted context hash",
            snapshot.injection_run_id
        )
    })?;
    let actual_context_hash = crate::context::context_output_fingerprint(injected_context);
    if persisted_context_hash != actual_context_hash {
        bail!(
            "coding-bench injected context differs from persisted injection_run_id={}",
            snapshot.injection_run_id
        );
    }
    verify_emitted_token_estimate(snapshot, injected_context)?;
    verify_persisted_item_mapping(conn, snapshot)?;
    Ok(())
}

fn verify_emitted_token_estimate(
    snapshot: &RememContextAuditSnapshot,
    injected_context: &str,
) -> Result<()> {
    let chars = u32::try_from(injected_context.chars().count()).map_err(|_| {
        anyhow::anyhow!("coding-bench injected context character count exceeds u32")
    })?;
    let emitted = chars.div_ceil(4);
    if snapshot.token_estimate != emitted {
        bail!(
            "coding-bench ContextAudit token estimate mismatch for injection_run_id={}: snapshot={} emitted={emitted}",
            snapshot.injection_run_id,
            snapshot.token_estimate
        );
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct PersistedInjectionAuditItem {
    item_kind: String,
    item_id: Option<i64>,
    memory_id: Option<i64>,
    channel: String,
    score: Option<f64>,
    status: String,
    drop_reason: Option<String>,
}

fn verify_persisted_item_mapping(
    conn: &Connection,
    snapshot: &RememContextAuditSnapshot,
) -> Result<()> {
    let (audit, _) = crate::context_bundle::persistence::decode_verified_context_audit_json(
        &snapshot.canonical_audit_json,
        snapshot.plan_schema_version,
    )?;
    let persisted_items = {
        let mut statement = conn.prepare(
            "SELECT item_kind, item_id, memory_id, channel, score, status, drop_reason
             FROM context_injection_items
             WHERE injection_run_id = ?1
             ORDER BY id",
        )?;
        let items = statement
            .query_map([&snapshot.injection_run_id], |row| {
                Ok(PersistedInjectionAuditItem {
                    item_kind: row.get(0)?,
                    item_id: row.get(1)?,
                    memory_id: row.get(2)?,
                    channel: row.get(3)?,
                    score: row.get(4)?,
                    status: row.get(5)?,
                    drop_reason: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        items
    };
    verify_persisted_items(snapshot, &audit, persisted_items)
}

fn verify_persisted_items(
    snapshot: &RememContextAuditSnapshot,
    audit: &ContextAudit,
    persisted_items: Vec<PersistedInjectionAuditItem>,
) -> Result<()> {
    let mut keyed_items = BTreeMap::new();
    let mut relevance_policy_count = 0_u32;
    let mut abstention_count = 0_u32;
    for item in persisted_items {
        if item.item_kind == "sessionstart_relevance_policy" {
            verify_relevance_policy_item(&item)?;
            relevance_policy_count += 1;
            continue;
        }
        if item.item_kind == "memory" && item.item_id.is_none() && item.memory_id.is_none() {
            verify_abstention_item(&item)?;
            abstention_count += 1;
            continue;
        }
        let Some(stable_key) = persisted_item_stable_key(&item)? else {
            bail!("linked ContextAudit item is missing its stable identity");
        };
        if keyed_items.insert(stable_key.clone(), item).is_some() {
            bail!(
                "coding-bench ContextAudit injection_run_id={} has duplicate linked item {stable_key}",
                snapshot.injection_run_id
            );
        }
    }
    if relevance_policy_count != 1 {
        bail!(
            "coding-bench ContextAudit injection_run_id={} must have exactly one linked relevance policy row",
            snapshot.injection_run_id
        );
    }
    if abstention_count > 1 {
        bail!(
            "coding-bench ContextAudit injection_run_id={} has duplicate linked abstention rows",
            snapshot.injection_run_id
        );
    }
    if keyed_items.len() != audit.entries.len() {
        bail!(
            "coding-bench ContextAudit injection_run_id={} linked item set does not match canonical audit",
            snapshot.injection_run_id
        );
    }
    for entry in &audit.entries {
        let Some(item) = keyed_items.remove(&entry.stable_key) else {
            bail!(
                "coding-bench ContextAudit injection_run_id={} is missing linked item {}",
                snapshot.injection_run_id,
                entry.stable_key
            );
        };
        let expected_status = if entry.selected {
            "injected"
        } else {
            "dropped"
        };
        let expected_drop_reason = if entry.selected {
            let expected_selected_reason = if entry.relevance_score.is_some() {
                "relevance_selected"
            } else {
                "channel_default_selected"
            };
            if entry.reason != expected_selected_reason {
                bail!(
                    "coding-bench ContextAudit injection_run_id={} selected item {} has non-canonical reason {}",
                    snapshot.injection_run_id,
                    entry.stable_key,
                    entry.reason
                );
            }
            None
        } else {
            Some(entry.reason.as_str())
        };
        if item.channel != channel_name(entry.channel)
            || item.score != entry.relevance_score
            || item.status != expected_status
            || item.drop_reason.as_deref() != expected_drop_reason
        {
            bail!(
                "coding-bench ContextAudit injection_run_id={} linked item {} differs from canonical audit: persisted={item:?} canonical={entry:?}",
                snapshot.injection_run_id,
                entry.stable_key
            );
        }
    }
    if !keyed_items.is_empty() {
        bail!(
            "coding-bench ContextAudit injection_run_id={} has extra linked items",
            snapshot.injection_run_id
        );
    }
    Ok(())
}

fn persisted_item_stable_key(item: &PersistedInjectionAuditItem) -> Result<Option<String>> {
    match item.item_kind.as_str() {
        "memory" => match (item.item_id, item.memory_id) {
            (Some(item_id), Some(memory_id)) if item_id == memory_id => {
                Ok(Some(format!("memory:{memory_id}")))
            }
            _ => bail!("linked memory item has non-canonical identity columns"),
        },
        "session_summary" => match (item.item_id, item.memory_id) {
            (Some(id), None) => Ok(Some(format!("session_summary:{id}"))),
            _ => bail!("linked session summary has non-canonical identity columns"),
        },
        "workstream" => match (item.item_id, item.memory_id) {
            (Some(id), None) => Ok(Some(format!("workstream:{id}"))),
            _ => bail!("linked workstream has non-canonical identity columns"),
        },
        other => bail!("linked ContextAudit item has unknown item_kind={other}"),
    }
}

fn verify_relevance_policy_item(item: &PersistedInjectionAuditItem) -> Result<()> {
    if item.item_id.is_some()
        || item.memory_id.is_some()
        || item.channel != "policy"
        || item.score.is_some_and(|score| !score.is_finite())
        || item.status != "injected"
        || item.drop_reason.is_some()
    {
        bail!("linked ContextAudit relevance policy row has non-canonical fields");
    }
    Ok(())
}

fn verify_abstention_item(item: &PersistedInjectionAuditItem) -> Result<()> {
    if item.channel != "memory"
        || item.score.is_some()
        || item.status != "abstained"
        || item.drop_reason.as_deref() != Some("no_relevant_context")
    {
        bail!("linked ContextAudit abstention row has non-canonical fields");
    }
    Ok(())
}

const fn channel_name(channel: ChannelKind) -> &'static str {
    match channel {
        ChannelKind::Preferences => "preferences",
        ChannelKind::Lessons => "lessons",
        ChannelKind::Core => "core",
        ChannelKind::Workstreams => "workstreams",
        ChannelKind::MemoryIndex => "index",
        ChannelKind::Sessions => "sessions",
    }
}

pub(crate) fn validate_run_context_audit(run: &RunReport) -> Result<()> {
    match run.condition {
        BenchCondition::RememSeededSessionStart => match run.context_audit_status {
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
    validate_supported_bundle_schema(snapshot.bundle_schema_version)?;
    validate_supported_bundle_schema(audit.schema_version)?;
    let candidates_considered = u32::try_from(audit.entries.len())
        .map_err(|_| anyhow::anyhow!("coding-bench ContextAudit entry count exceeds u32"))?;
    let selected_count = u32::try_from(audit.entries.iter().filter(|entry| entry.selected).count())
        .map_err(|_| anyhow::anyhow!("coding-bench ContextAudit selected count exceeds u32"))?;
    let dropped_count = candidates_considered - selected_count;
    if audit.candidates_considered != candidates_considered
        || audit.selected_count != selected_count
        || audit.dropped_count != dropped_count
    {
        bail!(
            "coding-bench ContextAudit entry counts do not match summary for injection_run_id={}",
            snapshot.injection_run_id
        );
    }
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

fn validate_supported_bundle_schema(version: u32) -> Result<()> {
    match version {
        SUPPORTED_CONTEXT_BUNDLE_SCHEMA_V1 => Ok(()),
        unsupported => {
            bail!("unsupported coding-bench ContextAudit bundle schema version {unsupported}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_bundle::{
        AuditEntry, ChannelKind, ContextAudit, ItemValidity, SourceKind,
        CONTEXT_BUNDLE_SCHEMA_VERSION,
    };

    fn audit_entry(stable_key: &str, selected: bool) -> AuditEntry {
        AuditEntry {
            stable_key: stable_key.to_string(),
            channel: ChannelKind::Core,
            source_kind: SourceKind::Canonical,
            validity: ItemValidity::Current,
            selected,
            reason: if selected {
                "relevance_selected".to_string()
            } else {
                "section_budget".to_string()
            },
            relevance_score: Some(0.75),
            token_estimate: if selected { 3 } else { 5 },
        }
    }

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
            entries: vec![
                audit_entry("memory:1", true),
                audit_entry("memory:2", false),
            ],
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

    fn persisted_item(
        id: i64,
        status: &str,
        drop_reason: Option<&str>,
    ) -> PersistedInjectionAuditItem {
        PersistedInjectionAuditItem {
            item_kind: "memory".to_string(),
            item_id: Some(id),
            memory_id: Some(id),
            channel: "core".to_string(),
            score: Some(0.75),
            status: status.to_string(),
            drop_reason: drop_reason.map(str::to_string),
        }
    }

    fn persisted_items() -> Vec<PersistedInjectionAuditItem> {
        vec![
            PersistedInjectionAuditItem {
                item_kind: "sessionstart_relevance_policy".to_string(),
                item_id: None,
                memory_id: None,
                channel: "policy".to_string(),
                score: Some(0.5),
                status: "injected".to_string(),
                drop_reason: None,
            },
            persisted_item(1, "injected", None),
            persisted_item(2, "dropped", Some("section_budget")),
        ]
    }

    fn abstention_item() -> PersistedInjectionAuditItem {
        PersistedInjectionAuditItem {
            item_kind: "memory".to_string(),
            item_id: None,
            memory_id: None,
            channel: "memory".to_string(),
            score: None,
            status: "abstained".to_string(),
            drop_reason: Some("no_relevant_context".to_string()),
        }
    }

    #[test]
    fn persisted_item_mapping_rejects_every_provenance_mutation() -> Result<()> {
        let snapshot = snapshot()?;
        let (audit, _) = crate::context_bundle::persistence::decode_verified_context_audit_json(
            &snapshot.canonical_audit_json,
            snapshot.plan_schema_version,
        )?;
        verify_persisted_items(&snapshot, &audit, persisted_items())?;
        let mut with_abstention = persisted_items();
        with_abstention.push(abstention_item());
        verify_persisted_items(&snapshot, &audit, with_abstention)?;

        let mut mutations = Vec::new();
        let mut missing = persisted_items();
        missing.pop();
        mutations.push(missing);
        let mut extra = persisted_items();
        extra.push(persisted_item(3, "dropped", Some("section_budget")));
        mutations.push(extra);
        let mut duplicate = persisted_items();
        duplicate.push(duplicate[1].clone());
        mutations.push(duplicate);
        let mut identity = persisted_items();
        identity[1].item_id = Some(9);
        mutations.push(identity);
        let mut score = persisted_items();
        score[1].score = Some(0.5);
        mutations.push(score);
        let mut status = persisted_items();
        status[1].status = "dropped".to_string();
        mutations.push(status);
        let mut reason = persisted_items();
        reason[2].drop_reason = Some("tampered".to_string());
        mutations.push(reason);
        let mut duplicate_policy = persisted_items();
        duplicate_policy.push(duplicate_policy[0].clone());
        mutations.push(duplicate_policy);
        let mut malformed_policy = persisted_items();
        malformed_policy[0].status = "dropped".to_string();
        mutations.push(malformed_policy);
        let mut malformed_abstention = persisted_items();
        let mut abstention = abstention_item();
        abstention.drop_reason = Some("tampered".to_string());
        malformed_abstention.push(abstention);
        mutations.push(malformed_abstention);
        let mut duplicate_abstention = persisted_items();
        duplicate_abstention.extend([abstention_item(), abstention_item()]);
        mutations.push(duplicate_abstention);
        for item_kind in ["session_summary", "workstream"] {
            let mut unexpected_identity = persisted_items();
            let mut item = persisted_item(3, "injected", None);
            item.item_kind = item_kind.to_string();
            unexpected_identity.push(item);
            mutations.push(unexpected_identity);
        }
        let mut unknown = persisted_items();
        unknown.push(PersistedInjectionAuditItem {
            item_kind: "unknown".to_string(),
            item_id: Some(3),
            memory_id: None,
            channel: "core".to_string(),
            score: None,
            status: "injected".to_string(),
            drop_reason: None,
        });
        mutations.push(unknown);

        for mutated in mutations {
            assert!(verify_persisted_items(&snapshot, &audit, mutated).is_err());
        }

        let mut selected_reason = audit.clone();
        selected_reason.entries[0].reason = "tampered".to_string();
        assert!(verify_persisted_items(&snapshot, &selected_reason, persisted_items()).is_err());
        Ok(())
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

        let mut unsupported_bundle = snapshot()?;
        let mut future_audit: ContextAudit =
            serde_json::from_str(&unsupported_bundle.canonical_audit_json)?;
        future_audit.schema_version = 2;
        let (canonical_audit_json, audit_hash) =
            crate::context_bundle::persistence::canonical_context_audit(
                &future_audit,
                unsupported_bundle.plan_schema_version,
            )?;
        unsupported_bundle.bundle_schema_version = 2;
        unsupported_bundle.canonical_audit_json = canonical_audit_json;
        unsupported_bundle.audit_hash = audit_hash;
        unsupported_bundle.injection_binding_hash = context_audit_binding_hash(
            &unsupported_bundle.injection_run_id,
            &unsupported_bundle.audit_hash,
        );
        assert!(verify_context_audit_snapshot(&unsupported_bundle)
            .unwrap_err()
            .to_string()
            .contains("unsupported coding-bench ContextAudit bundle schema version 2"));

        let mut inconsistent_entries = snapshot()?;
        let mut inconsistent_audit: ContextAudit =
            serde_json::from_str(&inconsistent_entries.canonical_audit_json)?;
        inconsistent_audit.entries.clear();
        let (canonical_audit_json, audit_hash) =
            crate::context_bundle::persistence::canonical_context_audit(
                &inconsistent_audit,
                inconsistent_entries.plan_schema_version,
            )?;
        inconsistent_entries.canonical_audit_json = canonical_audit_json;
        inconsistent_entries.audit_hash = audit_hash;
        inconsistent_entries.injection_binding_hash = context_audit_binding_hash(
            &inconsistent_entries.injection_run_id,
            &inconsistent_entries.audit_hash,
        );
        assert!(verify_context_audit_snapshot(&inconsistent_entries)
            .unwrap_err()
            .to_string()
            .contains("entry counts do not match summary"));
        Ok(())
    }

    #[test]
    fn emitted_context_token_estimate_is_hash_bound_aggregate() -> Result<()> {
        let mut snapshot = snapshot()?;
        let injected_context = "记忆系统有效";
        snapshot.token_estimate = 2;
        assert_ne!(
            audit_entry("memory:1", true).token_estimate,
            snapshot.token_estimate
        );
        verify_emitted_token_estimate(&snapshot, injected_context)?;

        snapshot.token_estimate = 3;
        let error = verify_emitted_token_estimate(&snapshot, injected_context).unwrap_err();
        assert!(error.to_string().contains("token estimate mismatch"));
        Ok(())
    }
}
