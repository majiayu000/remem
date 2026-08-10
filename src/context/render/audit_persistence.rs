//! Production SessionStart audit persistence and diagnostics.

use std::borrow::Cow;
use std::collections::HashSet;

use super::super::audit::{
    finalize_items_for_decision, record_context_injection, ContextAuditItem,
};
use super::super::injection_gate::{ContextGateAction, ContextGateDecision};
use super::super::invocation::ContextInvocation;

pub(super) fn persist_emission_audit(
    conn: &rusqlite::Connection,
    invocation: &ContextInvocation,
    decision: &ContextGateDecision,
    audit_items: &[ContextAuditItem],
    context_bundle: Option<&crate::context_bundle::ContextBundle>,
) {
    cleanup_persisted_bundle_audits(conn);
    let emitted_bundle = emitted_context_bundle(decision, audit_items, context_bundle);
    if let Err(error) = record_context_injection(
        conn,
        invocation,
        decision,
        audit_items,
        emitted_bundle.as_deref(),
    ) {
        crate::log::error(
            "context-audit",
            &format!(
                "failed to persist SessionStart audit for host={} project={} session={}: {error:#}",
                invocation.host.as_env_value(),
                invocation.project,
                invocation.session_id.as_deref().unwrap_or("<none>")
            ),
        );
    }
}

fn emitted_context_bundle<'a>(
    decision: &ContextGateDecision,
    audit_items: &[ContextAuditItem],
    context_bundle: Option<&'a crate::context_bundle::ContextBundle>,
) -> Option<Cow<'a, crate::context_bundle::ContextBundle>> {
    let bundle = context_bundle?;
    if decision.action == ContextGateAction::EmittedDelta
        || (decision.action == ContextGateAction::FailOpen
            && decision.retained_context_chars.is_some())
    {
        let selected_keys = finalize_items_for_decision(decision, audit_items)
            .into_iter()
            .filter(|item| item.status == "injected")
            .filter_map(|item| item.stable_key())
            .collect::<HashSet<_>>();
        let mut emitted = bundle.clone();
        crate::context_bundle::reseal_after_emission_gate(
            &mut emitted,
            &selected_keys,
            decision.output.chars().count(),
            "delta_preview",
            decision.output_truncated,
        );
        return Some(Cow::Owned(emitted));
    }
    match decision.action {
        ContextGateAction::Suppressed => None,
        ContextGateAction::Bypassed
        | ContextGateAction::EmittedFull
        | ContextGateAction::FailOpen
        | ContextGateAction::EmittedDelta => Some(Cow::Borrowed(bundle)),
    }
}

fn cleanup_persisted_bundle_audits(conn: &rusqlite::Connection) {
    let cutoff =
        super::super::injection_gate::retention_cutoff_epoch(chrono::Utc::now().timestamp());
    if let Err(error) = crate::context_bundle::cleanup_persisted_audits_before(conn, cutoff) {
        crate::log::error(
            "context-audit",
            &format!("retention cleanup failed: {error}"),
        );
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::context::host::HostKind;
    use crate::context_bundle::{
        AuditEntry, ChannelKind, ContextAudit, ContextBundle, ContextItem, DegradedMode,
        ItemValidity, SourceKind, TrustClass, CONTEXT_BUNDLE_SCHEMA_VERSION,
    };

    fn selected_bundle_item(id: i64) -> ContextItem {
        ContextItem {
            stable_key: format!("memory:{id}"),
            channel: ChannelKind::Preferences,
            title: format!("memory {id}"),
            text: format!("body {id}"),
            source_kind: SourceKind::Canonical,
            canonical_ref: Some(format!("memory:{id}")),
            projection_ref: None,
            evidence_refs: Vec::new(),
            validity: ItemValidity::Current,
            trust: TrustClass::Standard,
            project: Some("/repo".to_string()),
            branch: None,
        }
    }

    fn selected_audit_entry(id: i64) -> AuditEntry {
        AuditEntry {
            stable_key: format!("memory:{id}"),
            channel: ChannelKind::Preferences,
            source_kind: SourceKind::Canonical,
            validity: ItemValidity::Current,
            selected: true,
            reason: "selected_channel".to_string(),
            relevance_score: Some(1.0),
            token_estimate: 2,
        }
    }

    fn rendered_audit_item(id: i64, render_end_chars: usize) -> ContextAuditItem {
        ContextAuditItem {
            item_kind: "memory",
            item_id: Some(id),
            memory_id: Some(id),
            channel: "preferences",
            score: Some(1.0),
            render_order: Some(id),
            status: "injected",
            drop_reason: None,
            title: format!("memory {id}"),
            provenance: format!("src=memory:#{id}"),
            staleness: "fresh".to_string(),
            render_end_chars: Some(render_end_chars),
        }
    }

    fn selected_bundle() -> ContextBundle {
        let plan_hash = "a".repeat(64);
        ContextBundle {
            schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
            plan_hash: plan_hash.clone(),
            degraded_mode: DegradedMode::Full,
            preferences: vec![selected_bundle_item(1), selected_bundle_item(2)],
            failure_lessons: Vec::new(),
            current_truth: Vec::new(),
            workstreams: Vec::new(),
            memory_index: Vec::new(),
            recent_sessions: Vec::new(),
            audit: ContextAudit {
                schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
                policy_version: "retrieval_router_v2".to_string(),
                relevance_policy_version: "sessionstart_significant_token_v1".to_string(),
                plan_hash,
                degraded_mode: DegradedMode::Full,
                candidates_considered: 2,
                selected_count: 2,
                dropped_count: 0,
                token_estimate: 4,
                token_budget: 100,
                truncation_reason: None,
                entries: vec![selected_audit_entry(1), selected_audit_entry(2)],
            },
        }
    }

    #[test]
    fn delta_persistence_reseals_bundle_to_retained_items() -> Result<()> {
        let conn = rusqlite::Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let bundle = selected_bundle();
        let invocation = ContextInvocation {
            cwd: "/repo".to_string(),
            project: "/repo".to_string(),
            session_id: Some("delta-reseal".to_string()),
            transcript_path: None,
            source: Some("SessionStart".to_string()),
            host: HostKind::CodexCli,
            use_colors: false,
            debug: false,
            force: false,
            gate_mode: Some("delta".to_string()),
        };
        let decision = ContextGateDecision {
            output: "delta output".to_string(),
            action: ContextGateAction::EmittedDelta,
            reason: "changed_hash",
            key: Some("session:/repo:delta-reseal".to_string()),
            context_hash: Some("b".repeat(64)),
            output_mode: Some("delta"),
            retained_context_chars: Some(100),
            output_truncated: true,
        };

        persist_emission_audit(
            &conn,
            &invocation,
            &decision,
            &[rendered_audit_item(1, 50), rendered_audit_item(2, 150)],
            Some(&bundle),
        );

        let run_id: String = conn.query_row(
            "SELECT injection_run_id FROM context_injection_items
             WHERE session_id = 'delta-reseal' LIMIT 1",
            [],
            |row| row.get(0),
        )?;
        let statuses: (i64, i64) = conn.query_row(
            "SELECT SUM(status = 'injected'),
                    SUM(status = 'dropped' AND drop_reason = 'delta_preview')
             FROM context_injection_items WHERE injection_run_id = ?1",
            [&run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(statuses, (1, 1));
        let persisted =
            crate::context_bundle::persistence::load_verified_context_bundle_audit(&conn, &run_id)?
                .ok_or_else(|| anyhow::anyhow!("missing resealed delta audit"))?;
        assert_eq!(persisted.audit.selected_count, 1);
        assert_eq!(persisted.audit.dropped_count, 1);
        assert_eq!(persisted.audit.token_estimate, 3);
        assert_eq!(
            persisted.audit.truncation_reason.as_deref(),
            Some("delta_preview")
        );
        assert!(persisted.audit.entries[0].selected);
        assert!(!persisted.audit.entries[1].selected);
        assert_eq!(persisted.audit.entries[1].reason, "delta_preview");
        Ok(())
    }

    #[test]
    fn delta_reseal_records_output_only_truncation() -> Result<()> {
        let bundle = selected_bundle();
        let decision = ContextGateDecision {
            output: "truncated delta output".to_string(),
            action: ContextGateAction::EmittedDelta,
            reason: "changed_hash",
            key: None,
            context_hash: None,
            output_mode: Some("delta"),
            retained_context_chars: Some(200),
            output_truncated: true,
        };

        let emitted = emitted_context_bundle(
            &decision,
            &[rendered_audit_item(1, 50), rendered_audit_item(2, 150)],
            Some(&bundle),
        )
        .ok_or_else(|| anyhow::anyhow!("delta bundle should be resealed"))?;

        assert_eq!(emitted.audit.selected_count, 2);
        assert_eq!(emitted.audit.dropped_count, 0);
        assert_eq!(
            emitted.audit.truncation_reason.as_deref(),
            Some("delta_preview")
        );
        Ok(())
    }

    #[test]
    fn delta_gate_write_fail_open_reseals_to_emitted_items() -> Result<()> {
        let bundle = selected_bundle();
        let decision = ContextGateDecision {
            output: "fail-open delta output".to_string(),
            action: ContextGateAction::FailOpen,
            reason: "gate_write",
            key: None,
            context_hash: None,
            output_mode: Some("delta"),
            retained_context_chars: Some(100),
            output_truncated: true,
        };

        let emitted = emitted_context_bundle(
            &decision,
            &[rendered_audit_item(1, 50), rendered_audit_item(2, 150)],
            Some(&bundle),
        )
        .ok_or_else(|| anyhow::anyhow!("fail-open delta bundle should be resealed"))?;

        assert_eq!(emitted.audit.selected_count, 1);
        assert_eq!(emitted.audit.dropped_count, 1);
        assert_eq!(emitted.audit.entries[1].reason, "delta_preview");
        assert_eq!(
            emitted.audit.truncation_reason.as_deref(),
            Some("delta_preview")
        );
        Ok(())
    }

    #[test]
    fn persistence_path_cleans_old_bundle_audits_when_gate_is_off() -> Result<()> {
        let conn = rusqlite::Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        conn.execute(
            "INSERT INTO context_injection_items
             (injection_run_id, host, project, injection_key, output_mode, decision,
              item_kind, channel, status, injected_at_epoch)
             VALUES ('old-run', 'codex-cli', '/repo', 'key', 'full', 'emitted',
                     'memory', 'core', 'injected', 1)",
            [],
        )?;
        conn.execute(
            "INSERT INTO context_bundle_audits
             (injection_run_id, bundle_schema_version, plan_schema_version,
              policy_version, relevance_policy_version, plan_hash, audit_hash,
              degraded_mode, candidates_considered, selected_count, dropped_count,
              token_budget, token_estimate, truncation_reason, audit_json,
              created_at_epoch)
             VALUES ('old-run', 1, 1, 'router-v1', 'relevance-v1', ?1, ?1,
                     'full', 0, 0, 0, 1, 0, NULL, '{}', 1)",
            ["a".repeat(64)],
        )?;
        conn.execute(
            "INSERT INTO context_injection_items
             (injection_run_id, host, project, injection_key, output_mode, decision,
              item_kind, channel, status, injected_at_epoch)
             VALUES ('fresh-run', 'codex-cli', '/repo', 'key', 'full', 'emitted',
                     'memory', 'core', 'injected', ?1)",
            [chrono::Utc::now().timestamp()],
        )?;
        conn.execute(
            "INSERT INTO context_bundle_audits
             (injection_run_id, bundle_schema_version, plan_schema_version,
              policy_version, relevance_policy_version, plan_hash, audit_hash,
              degraded_mode, candidates_considered, selected_count, dropped_count,
              token_budget, token_estimate, truncation_reason, audit_json,
              created_at_epoch)
             VALUES ('fresh-run', 1, 1, 'router-v1', 'relevance-v1', ?1, ?1,
                     'full', 0, 0, 0, 1, 0, NULL, '{}', ?2)",
            rusqlite::params!["b".repeat(64), chrono::Utc::now().timestamp()],
        )?;
        let invocation = ContextInvocation {
            cwd: "/repo".to_string(),
            project: "/repo".to_string(),
            session_id: Some("gate-off".to_string()),
            transcript_path: None,
            source: Some("SessionStart".to_string()),
            host: HostKind::CodexCli,
            use_colors: false,
            debug: false,
            force: false,
            gate_mode: Some("off".to_string()),
        };
        let decision = ContextGateDecision {
            output: "context".to_string(),
            action: ContextGateAction::Bypassed,
            reason: "gate_off",
            key: None,
            context_hash: None,
            output_mode: Some("bypassed"),
            retained_context_chars: None,
            output_truncated: false,
        };
        let item = ContextAuditItem {
            item_kind: "memory",
            item_id: Some(1),
            memory_id: Some(1),
            channel: "core",
            score: None,
            render_order: Some(1),
            status: "injected",
            drop_reason: None,
            title: "fixture".to_string(),
            provenance: "src=memory:#1".to_string(),
            staleness: "fresh".to_string(),
            render_end_chars: None,
        };

        persist_emission_audit(&conn, &invocation, &decision, &[item], None);

        let old_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM context_bundle_audits
             WHERE injection_run_id = 'old-run'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(old_count, 0);
        let fresh_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM context_bundle_audits
             WHERE injection_run_id = 'fresh-run'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(fresh_count, 1);
        let new_item_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM context_injection_items
             WHERE session_id = 'gate-off'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(new_item_count, 1);
        Ok(())
    }
}
