//! Audit construction for Context Bundle v1: every candidate considered
//! ends up in exactly one [`AuditEntry`] with a machine-readable reason.

use std::collections::HashMap;

use crate::retrieval_router::RetrievalPlan;

use super::domain::{
    AuditEntry, ContextAudit, ContextItem, DegradedMode, CONTEXT_BUNDLE_SCHEMA_VERSION,
};
use super::policy::estimate_tokens;

#[derive(Debug, Default)]
pub(super) struct AuditBuilder {
    entries: Vec<AuditEntry>,
    scores: HashMap<String, f64>,
    truncation_reason: Option<String>,
}

impl AuditBuilder {
    pub(super) fn record_score(&mut self, stable_key: &str, score: f64) {
        self.scores.insert(stable_key.to_string(), score);
    }

    pub(super) fn selected(&mut self, item: &ContextItem, reason: &str) {
        self.push(item, true, reason);
    }

    pub(super) fn dropped(&mut self, item: &ContextItem, reason: &str) {
        self.push(item, false, reason);
    }

    pub(super) fn set_truncation_reason(&mut self, reason: &str) {
        if self.truncation_reason.is_none() {
            self.truncation_reason = Some(reason.to_string());
        }
    }

    fn push(&mut self, item: &ContextItem, selected: bool, reason: &str) {
        self.entries.push(AuditEntry {
            stable_key: item.stable_key.clone(),
            channel: item.channel,
            source_kind: item.source_kind,
            validity: item.validity,
            selected,
            reason: reason.to_string(),
            relevance_score: self.scores.get(&item.stable_key).copied(),
            token_estimate: estimate_tokens(&item.text),
        });
    }

    pub(super) fn finalize(
        mut self,
        plan: &RetrievalPlan,
        degraded_mode: DegradedMode,
    ) -> ContextAudit {
        // Deterministic order regardless of drop/select interleaving.
        self.entries.sort_by(|left, right| {
            (left.channel, &left.stable_key).cmp(&(right.channel, &right.stable_key))
        });
        let selected_count = self.entries.iter().filter(|entry| entry.selected).count() as u32;
        let token_estimate = self
            .entries
            .iter()
            .filter(|entry| entry.selected)
            .map(|entry| entry.token_estimate)
            .sum();
        let candidates_considered = self.entries.len() as u32;
        ContextAudit {
            schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
            policy_version: plan.policy_version.clone(),
            relevance_policy_version: plan.relevance_policy_version.clone(),
            plan_hash: plan.plan_hash.clone(),
            degraded_mode,
            candidates_considered,
            selected_count,
            dropped_count: candidates_considered - selected_count,
            token_estimate,
            token_budget: plan.section_budgets.total_tokens,
            truncation_reason: self.truncation_reason,
            entries: self.entries,
        }
    }
}
