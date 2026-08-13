//! Durable, payload-free ContextAudit persistence for production SessionStart.

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::retrieval_router::RETRIEVAL_PLAN_SCHEMA_VERSION;

use super::{ContextAudit, ContextBundle, DegradedMode};

pub(crate) const PERSISTED_PLAN_SCHEMA_V1: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PersistedContextBundleAudit {
    pub injection_run_id: String,
    pub bundle_schema_version: u32,
    pub plan_schema_version: u32,
    pub audit_hash: String,
    pub canonical_audit_json: String,
    pub audit: ContextAudit,
    pub created_at_epoch: i64,
}

#[derive(Debug)]
struct StoredAuditRow {
    injection_run_id: String,
    bundle_schema_version: u32,
    plan_schema_version: u32,
    policy_version: String,
    relevance_policy_version: String,
    plan_hash: String,
    audit_hash: String,
    degraded_mode: String,
    candidates_considered: u32,
    selected_count: u32,
    dropped_count: u32,
    token_budget: u32,
    token_estimate: u32,
    truncation_reason: Option<String>,
    audit_json: String,
    created_at_epoch: i64,
}

pub(crate) fn persist_context_bundle_audit(
    conn: &Connection,
    injection_run_id: &str,
    bundle: &ContextBundle,
    created_at_epoch: i64,
) -> Result<String> {
    validate_bundle_summary(bundle)?;
    validate_persisted_plan_schema_version(RETRIEVAL_PLAN_SCHEMA_VERSION)?;
    let (audit_json, audit_hash) =
        canonical_context_audit(&bundle.audit, RETRIEVAL_PLAN_SCHEMA_VERSION)?;

    if let Some(existing) = load_verified_context_bundle_audit(conn, injection_run_id)? {
        if existing.audit_hash == audit_hash {
            return Ok(audit_hash);
        }
        bail!(
            "context bundle audit integrity conflict for injection_run_id={injection_run_id}: stored_hash={} incoming_hash={audit_hash}",
            existing.audit_hash
        );
    }

    let linked_items: i64 = conn.query_row(
        "SELECT COUNT(*) FROM context_injection_items WHERE injection_run_id = ?1",
        [injection_run_id],
        |row| row.get(0),
    )?;
    if linked_items == 0 {
        bail!(
            "context bundle audit has no context_injection_items link for injection_run_id={injection_run_id}"
        );
    }

    conn.execute(
        "INSERT INTO context_bundle_audits
         (injection_run_id, bundle_schema_version, plan_schema_version,
          policy_version, relevance_policy_version, plan_hash, audit_hash,
          degraded_mode, candidates_considered, selected_count, dropped_count,
          token_budget, token_estimate, truncation_reason, audit_json,
          created_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                 ?14, ?15, ?16)",
        params![
            injection_run_id,
            bundle.schema_version,
            RETRIEVAL_PLAN_SCHEMA_VERSION,
            bundle.audit.policy_version,
            bundle.audit.relevance_policy_version,
            bundle.plan_hash,
            audit_hash,
            degraded_mode_name(bundle.degraded_mode),
            bundle.audit.candidates_considered,
            bundle.audit.selected_count,
            bundle.audit.dropped_count,
            bundle.audit.token_budget,
            bundle.audit.token_estimate,
            bundle.audit.truncation_reason,
            audit_json,
            created_at_epoch,
        ],
    )
    .with_context(|| {
        format!("insert context bundle audit for injection_run_id={injection_run_id}")
    })?;
    Ok(audit_hash)
}

pub(crate) fn load_verified_context_bundle_audit(
    conn: &Connection,
    injection_run_id: &str,
) -> Result<Option<PersistedContextBundleAudit>> {
    let row = conn
        .query_row(
            "SELECT injection_run_id, bundle_schema_version, plan_schema_version,
                    policy_version, relevance_policy_version, plan_hash, audit_hash,
                    degraded_mode, candidates_considered, selected_count, dropped_count,
                    token_budget, token_estimate, truncation_reason, audit_json,
                    created_at_epoch
             FROM context_bundle_audits
             WHERE injection_run_id = ?1",
            [injection_run_id],
            |row| {
                Ok(StoredAuditRow {
                    injection_run_id: row.get(0)?,
                    bundle_schema_version: row.get(1)?,
                    plan_schema_version: row.get(2)?,
                    policy_version: row.get(3)?,
                    relevance_policy_version: row.get(4)?,
                    plan_hash: row.get(5)?,
                    audit_hash: row.get(6)?,
                    degraded_mode: row.get(7)?,
                    candidates_considered: row.get(8)?,
                    selected_count: row.get(9)?,
                    dropped_count: row.get(10)?,
                    token_budget: row.get(11)?,
                    token_estimate: row.get(12)?,
                    truncation_reason: row.get(13)?,
                    audit_json: row.get(14)?,
                    created_at_epoch: row.get(15)?,
                })
            },
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    validate_persisted_plan_schema_version(row.plan_schema_version)?;

    let linked_items: i64 = conn.query_row(
        "SELECT COUNT(*) FROM context_injection_items WHERE injection_run_id = ?1",
        [&row.injection_run_id],
        |item_row| item_row.get(0),
    )?;
    if linked_items == 0 {
        bail!(
            "context bundle audit integrity failure: missing context_injection_items for injection_run_id={}",
            row.injection_run_id
        );
    }

    let (audit, actual_hash) =
        decode_verified_context_audit_json(&row.audit_json, row.plan_schema_version).with_context(
            || {
                format!(
                    "verify persisted ContextAudit for injection_run_id={}",
                    row.injection_run_id
                )
            },
        )?;
    if actual_hash != row.audit_hash {
        bail!(
            "context bundle audit hash mismatch for injection_run_id={}: stored={} actual={actual_hash}",
            row.injection_run_id,
            row.audit_hash
        );
    }
    verify_stored_summary(&row, &audit)?;

    Ok(Some(PersistedContextBundleAudit {
        injection_run_id: row.injection_run_id,
        bundle_schema_version: row.bundle_schema_version,
        plan_schema_version: row.plan_schema_version,
        audit_hash: row.audit_hash,
        canonical_audit_json: row.audit_json,
        audit,
        created_at_epoch: row.created_at_epoch,
    }))
}

pub(crate) fn cleanup_persisted_audits_before(
    conn: &Connection,
    cutoff_epoch: i64,
) -> Result<usize> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_master
             WHERE type = 'table' AND name = 'context_bundle_audits'
         )",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(0);
    }
    Ok(conn.execute(
        "DELETE FROM context_bundle_audits WHERE created_at_epoch < ?1",
        [cutoff_epoch],
    )?)
}

fn validate_bundle_summary(bundle: &ContextBundle) -> Result<()> {
    if bundle.schema_version != bundle.audit.schema_version {
        bail!("ContextBundle and ContextAudit schema versions differ");
    }
    if bundle.plan_hash != bundle.audit.plan_hash {
        bail!("ContextBundle and ContextAudit plan hashes differ");
    }
    if bundle.degraded_mode != bundle.audit.degraded_mode {
        bail!("ContextBundle and ContextAudit degraded modes differ");
    }
    if bundle.audit.selected_count + bundle.audit.dropped_count
        != bundle.audit.candidates_considered
    {
        bail!("ContextAudit selection counts are inconsistent");
    }
    Ok(())
}

fn verify_stored_summary(row: &StoredAuditRow, audit: &ContextAudit) -> Result<()> {
    let expected_mode = degraded_mode_name(audit.degraded_mode);
    let matches = row.bundle_schema_version == audit.schema_version
        && row.policy_version == audit.policy_version
        && row.relevance_policy_version == audit.relevance_policy_version
        && row.plan_hash == audit.plan_hash
        && row.degraded_mode == expected_mode
        && row.candidates_considered == audit.candidates_considered
        && row.selected_count == audit.selected_count
        && row.dropped_count == audit.dropped_count
        && row.token_budget == audit.token_budget
        && row.token_estimate == audit.token_estimate
        && row.truncation_reason == audit.truncation_reason;
    if !matches {
        bail!(
            "context bundle audit summary mismatch for injection_run_id={}",
            row.injection_run_id
        );
    }
    Ok(())
}

fn validate_persisted_plan_schema_version(version: u32) -> Result<()> {
    match version {
        PERSISTED_PLAN_SCHEMA_V1 => Ok(()),
        unsupported => bail!("unsupported persisted retrieval plan schema version {unsupported}"),
    }
}

fn decode_persisted_audit(version: u32, value: Value) -> Result<ContextAudit> {
    match version {
        PERSISTED_PLAN_SCHEMA_V1 => Ok(serde_json::from_value(value)?),
        unsupported => bail!("unsupported persisted retrieval plan schema version {unsupported}"),
    }
}

pub(crate) fn canonical_context_audit(
    audit: &ContextAudit,
    plan_schema_version: u32,
) -> Result<(String, String)> {
    validate_persisted_plan_schema_version(plan_schema_version)?;
    let value = serde_json::to_value(audit)?;
    let bytes = canonical_json_bytes(&value)?;
    let json = String::from_utf8(bytes.clone()).context("canonical ContextAudit is UTF-8")?;
    let hash = persisted_audit_hash(plan_schema_version, &value)?;
    Ok((json, hash))
}

pub(crate) fn decode_verified_context_audit_json(
    audit_json: &str,
    plan_schema_version: u32,
) -> Result<(ContextAudit, String)> {
    validate_persisted_plan_schema_version(plan_schema_version)?;
    let value: Value = serde_json::from_str(audit_json).context("parse canonical ContextAudit")?;
    let canonical = canonical_json_bytes(&value)?;
    if canonical.as_slice() != audit_json.as_bytes() {
        bail!("ContextAudit JSON is not canonical");
    }
    let hash = persisted_audit_hash(plan_schema_version, &value)?;
    let audit = decode_persisted_audit(plan_schema_version, value)
        .context("decode canonical ContextAudit")?;
    Ok((audit, hash))
}

fn persisted_audit_hash(plan_schema_version: u32, audit: &Value) -> Result<String> {
    let envelope = serde_json::json!({
        "audit": audit,
        "plan_schema_version": plan_schema_version,
    });
    Ok(format!(
        "{:x}",
        Sha256::digest(canonical_json_bytes(&envelope)?)
    ))
}

fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    write_canonical_json(value, &mut output)?;
    Ok(output)
}

fn write_canonical_json(value: &Value, output: &mut Vec<u8>) -> Result<()> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            serde_json::to_writer(output, value)?;
        }
        Value::Array(values) => {
            output.push(b'[');
            for (index, item) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical_json(item, output)?;
            }
            output.push(b']');
        }
        Value::Object(object) => {
            output.push(b'{');
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, key)?;
                output.push(b':');
                write_canonical_json(&object[*key], output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

fn degraded_mode_name(mode: DegradedMode) -> &'static str {
    match mode {
        DegradedMode::Full => "full",
        DegradedMode::CanonicalOnly => "canonical_only",
        DegradedMode::Blocked => "blocked",
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rusqlite::{params, Connection};

    use super::*;
    use crate::context_bundle::{
        AuditEntry, ChannelKind, ContextItem, ItemValidity, SourceKind, TrustClass,
        CONTEXT_BUNDLE_SCHEMA_VERSION,
    };

    const PLAN_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn database() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    fn insert_item_link(conn: &Connection, run_id: &str) -> Result<()> {
        conn.execute(
            "INSERT INTO context_injection_items
             (injection_run_id, host, project, injection_key, output_mode, decision,
              item_kind, channel, status, injected_at_epoch)
             VALUES (?1, 'codex-cli', 'project', 'key', 'full', 'emitted',
                     'memory', 'core', 'injected', 100)",
            [run_id],
        )?;
        Ok(())
    }

    fn bundle(secret_payload: &str) -> ContextBundle {
        let audit = ContextAudit {
            schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
            policy_version: "retrieval_router_v2".into(),
            relevance_policy_version: "sessionstart_significant_token_v1".into(),
            plan_hash: PLAN_HASH.into(),
            degraded_mode: DegradedMode::Full,
            candidates_considered: 1,
            selected_count: 1,
            dropped_count: 0,
            token_estimate: 2,
            token_budget: 100,
            truncation_reason: None,
            shadow_comparison: Vec::new(),
            entries: vec![AuditEntry {
                stable_key: "memory:7".into(),
                channel: ChannelKind::Core,
                source_kind: SourceKind::Canonical,
                validity: ItemValidity::Current,
                selected: true,
                reason: "selected_channel".into(),
                relevance_score: Some(0.75),
                token_estimate: 2,
            }],
        };
        ContextBundle {
            schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
            plan_hash: PLAN_HASH.into(),
            degraded_mode: DegradedMode::Full,
            preferences: Vec::new(),
            failure_lessons: Vec::new(),
            current_truth: vec![ContextItem {
                stable_key: "memory:7".into(),
                channel: ChannelKind::Core,
                title: format!("title {secret_payload}"),
                text: format!("body {secret_payload}"),
                source_kind: SourceKind::Canonical,
                canonical_ref: Some("memory:7".into()),
                projection_ref: None,
                evidence_refs: Vec::new(),
                validity: ItemValidity::Current,
                trust: TrustClass::Standard,
                project: Some("project".into()),
                branch: None,
            }],
            workstreams: Vec::new(),
            memory_index: Vec::new(),
            recent_sessions: Vec::new(),
            audit,
        }
    }

    #[test]
    fn persists_verifies_and_does_not_leak_bundle_payload() -> Result<()> {
        let conn = database()?;
        insert_item_link(&conn, "run-1")?;
        let bundle = bundle("fixture-secret-payload");

        let hash = persist_context_bundle_audit(&conn, "run-1", &bundle, 100)?;
        let stored = load_verified_context_bundle_audit(&conn, "run-1")?
            .ok_or_else(|| anyhow::anyhow!("missing persisted audit"))?;
        assert_eq!(stored.audit_hash, hash);
        assert_eq!(stored.audit, bundle.audit);

        let persisted_text: String = conn.query_row(
            "SELECT policy_version || relevance_policy_version || plan_hash ||
                    audit_hash || degraded_mode || COALESCE(truncation_reason, '') || audit_json
             FROM context_bundle_audits WHERE injection_run_id = 'run-1'",
            [],
            |row| row.get(0),
        )?;
        assert!(!persisted_text.contains("fixture-secret-payload"));
        assert!(!persisted_text.contains("title "));
        assert!(!persisted_text.contains("body "));
        Ok(())
    }

    #[test]
    fn same_run_retry_is_idempotent_and_conflict_fails() -> Result<()> {
        let conn = database()?;
        insert_item_link(&conn, "run-retry")?;
        let original = bundle("not-persisted");
        let first = persist_context_bundle_audit(&conn, "run-retry", &original, 100)?;
        let repeated = persist_context_bundle_audit(&conn, "run-retry", &original, 100)?;
        assert_eq!(first, repeated);
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM context_bundle_audits WHERE injection_run_id = 'run-retry'",
                [],
                |row| row.get::<_, i64>(0),
            )?,
            1
        );

        let mut conflicting = original;
        conflicting.audit.token_estimate = 3;
        let error = persist_context_bundle_audit(&conn, "run-retry", &conflicting, 100)
            .expect_err("conflicting retry must fail");
        assert!(error.to_string().contains("integrity conflict"));
        Ok(())
    }

    #[test]
    fn verified_read_detects_audit_json_and_summary_tampering() -> Result<()> {
        let conn = database()?;
        insert_item_link(&conn, "run-tamper")?;
        persist_context_bundle_audit(&conn, "run-tamper", &bundle("not-persisted"), 100)?;

        conn.execute_batch("DROP TRIGGER context_bundle_audits_immutable_update;")?;
        conn.execute(
            "UPDATE context_bundle_audits
             SET audit_json = replace(audit_json, '\"token_estimate\":2', '\"token_estimate\":3')
             WHERE injection_run_id = 'run-tamper'",
            [],
        )?;
        let error = load_verified_context_bundle_audit(&conn, "run-tamper")
            .expect_err("tampered JSON must fail verification");
        assert!(error.to_string().contains("hash mismatch"));

        conn.execute(
            "UPDATE context_bundle_audits SET audit_json = ?1, token_estimate = 99
             WHERE injection_run_id = 'run-tamper'",
            params![canonical_context_audit(&bundle("ignored").audit, PERSISTED_PLAN_SCHEMA_V1)?.0],
        )?;
        let error = load_verified_context_bundle_audit(&conn, "run-tamper")
            .expect_err("tampered summary must fail verification");
        assert!(error.to_string().contains("summary mismatch"));
        Ok(())
    }

    #[test]
    fn verified_read_dispatches_on_persisted_plan_schema_version() -> Result<()> {
        let conn = database()?;
        insert_item_link(&conn, "run-plan-version")?;
        persist_context_bundle_audit(&conn, "run-plan-version", &bundle("not-persisted"), 100)?;

        conn.execute_batch("DROP TRIGGER context_bundle_audits_immutable_update;")?;
        conn.execute(
            "UPDATE context_bundle_audits
             SET plan_schema_version = 2
             WHERE injection_run_id = 'run-plan-version'",
            [],
        )?;
        let error = load_verified_context_bundle_audit(&conn, "run-plan-version")
            .expect_err("unsupported persisted plan schema must fail explicitly");
        assert!(error
            .to_string()
            .contains("unsupported persisted retrieval plan schema version 2"));
        Ok(())
    }

    #[test]
    fn verified_decode_rejects_unknown_v1_audit_fields() -> Result<()> {
        let audit = bundle("not-persisted").audit;
        let mut top_level = serde_json::to_value(&audit)?;
        top_level
            .as_object_mut()
            .expect("ContextAudit serializes as an object")
            .insert(
                "task_prompt".to_string(),
                Value::String("secret".to_string()),
            );
        let top_level_json = String::from_utf8(canonical_json_bytes(&top_level)?)?;
        let error = decode_verified_context_audit_json(&top_level_json, PERSISTED_PLAN_SCHEMA_V1)
            .expect_err("unknown top-level audit field must fail closed");
        assert!(error.to_string().contains("decode canonical ContextAudit"));

        let mut entry_level = serde_json::to_value(&audit)?;
        entry_level["entries"][0]
            .as_object_mut()
            .expect("AuditEntry serializes as an object")
            .insert(
                "memory_text".to_string(),
                Value::String("secret".to_string()),
            );
        let entry_level_json = String::from_utf8(canonical_json_bytes(&entry_level)?)?;
        let error = decode_verified_context_audit_json(&entry_level_json, PERSISTED_PLAN_SCHEMA_V1)
            .expect_err("unknown AuditEntry field must fail closed");
        assert!(error.to_string().contains("decode canonical ContextAudit"));
        Ok(())
    }

    #[test]
    fn persisted_v1_hash_envelope_is_frozen() -> Result<()> {
        let (_, hash) =
            canonical_context_audit(&bundle("not-persisted").audit, PERSISTED_PLAN_SCHEMA_V1)?;
        assert_eq!(
            hash,
            "4e9818dcbb42fd97cca6e86e4975058777dd278cb329fcf7f228443a29191b06"
        );
        Ok(())
    }

    #[test]
    fn retention_cleanup_deletes_only_expired_audits() -> Result<()> {
        let conn = database()?;
        for (run_id, created) in [("old-run", 100), ("new-run", 200)] {
            insert_item_link(&conn, run_id)?;
            persist_context_bundle_audit(&conn, run_id, &bundle("not-persisted"), created)?;
        }
        assert_eq!(cleanup_persisted_audits_before(&conn, 150)?, 1);
        assert!(load_verified_context_bundle_audit(&conn, "old-run")?.is_none());
        assert!(load_verified_context_bundle_audit(&conn, "new-run")?.is_some());
        Ok(())
    }
}
