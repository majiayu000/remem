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
