//! Production SessionStart audit persistence and diagnostics.

use super::super::audit::{record_context_injection, ContextAuditItem};
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
    let emitted_bundle = (!matches!(decision.action, ContextGateAction::Suppressed))
        .then_some(context_bundle)
        .flatten();
    if let Err(error) =
        record_context_injection(conn, invocation, decision, audit_items, emitted_bundle)
    {
        crate::log::error(
            "context-audit",
            &format!(
                "failed to persist SessionStart audit for host={} project={} session={}: {error}",
                invocation.host.as_env_value(),
                invocation.project,
                invocation.session_id.as_deref().unwrap_or("<none>")
            ),
        );
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
