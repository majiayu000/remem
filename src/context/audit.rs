use anyhow::Result;
use rusqlite::params;
use std::collections::{HashMap, HashSet};

use crate::memory::{
    age_staleness_label, memory_staleness as shared_memory_staleness, Memory, MemoryStalenessLabel,
};

use super::injection_gate::{ContextGateAction, ContextGateDecision};
use super::invocation::ContextInvocation;
use super::relevance::{
    memory_stable_key, session_stable_key, SessionStartRelevancePlan,
    SESSIONSTART_RELEVANCE_POLICY_VERSION,
};
use super::types::{LoadedContext, SessionSummaryBrief};

#[derive(Debug, Clone, PartialEq)]
pub(in crate::context) struct ContextAuditItem {
    pub item_kind: &'static str,
    pub item_id: Option<i64>,
    pub memory_id: Option<i64>,
    pub channel: &'static str,
    pub score: Option<f64>,
    pub render_order: Option<i64>,
    pub status: &'static str,
    pub drop_reason: Option<&'static str>,
    pub title: String,
    pub provenance: String,
    pub staleness: String,
}

impl ContextAuditItem {
    pub fn injected_memory(memory: &Memory, channel: &'static str, render_order: i64) -> Self {
        Self::memory_item(memory, channel, Some(render_order), "injected", None)
    }

    pub fn injected_memory_with_labels(
        memory: &Memory,
        channel: &'static str,
        render_order: i64,
        staleness_labels: &HashMap<i64, MemoryStalenessLabel>,
    ) -> Self {
        let mut item = Self::injected_memory(memory, channel, render_order);
        item.staleness =
            memory_staleness_with_labels(memory, chrono::Utc::now().timestamp(), staleness_labels);
        item
    }

    pub fn dropped_memory(memory: &Memory, channel: &'static str, reason: &'static str) -> Self {
        Self::memory_item(memory, channel, None, "dropped", Some(reason))
    }

    fn with_score(mut self, score: f64) -> Self {
        self.score = Some(score);
        self
    }

    pub fn abstained_memory(reason: &'static str) -> Self {
        Self {
            item_kind: "memory",
            item_id: None,
            memory_id: None,
            channel: "memory",
            score: None,
            render_order: None,
            status: "abstained",
            drop_reason: Some(reason),
            title: "memory context abstained".to_string(),
            provenance: "src=memory".to_string(),
            staleness: "staleness=none".to_string(),
        }
    }

    pub fn injected_workstream(
        id: i64,
        title: &str,
        render_order: i64,
        updated_at_epoch: i64,
    ) -> Self {
        Self {
            item_kind: "workstream",
            item_id: Some(id),
            memory_id: None,
            channel: "workstreams",
            score: None,
            render_order: Some(render_order),
            status: "injected",
            drop_reason: None,
            title: title.to_string(),
            provenance: format!("src=workstream:#{id}"),
            staleness: age_staleness_label(updated_at_epoch, chrono::Utc::now().timestamp()),
        }
    }

    pub fn dropped_workstream(
        id: i64,
        title: &str,
        updated_at_epoch: i64,
        reason: &'static str,
    ) -> Self {
        Self {
            item_kind: "workstream",
            item_id: Some(id),
            memory_id: None,
            channel: "workstreams",
            score: None,
            render_order: None,
            status: "dropped",
            drop_reason: Some(reason),
            title: title.to_string(),
            provenance: format!("src=workstream:#{id}"),
            staleness: age_staleness_label(updated_at_epoch, chrono::Utc::now().timestamp()),
        }
    }

    fn session_summary(
        summary: &SessionSummaryBrief,
        render_order: Option<i64>,
        status: &'static str,
        drop_reason: Option<&'static str>,
        score: f64,
    ) -> Self {
        Self {
            item_kind: "session_summary",
            item_id: Some(summary.id),
            memory_id: None,
            channel: "sessions",
            score: Some(score),
            render_order,
            status,
            drop_reason,
            title: super::format::truncate_chars_with_ellipsis(
                &super::format::inline_context_text(&summary.request),
                160,
            ),
            provenance: format!("src=session_summary:#{}", summary.id),
            staleness: age_staleness_label(
                summary.created_at_epoch,
                chrono::Utc::now().timestamp(),
            ),
        }
    }

    fn relevance_policy(plan: &SessionStartRelevancePlan) -> Self {
        Self {
            item_kind: "sessionstart_relevance_policy",
            item_id: None,
            memory_id: None,
            channel: "policy",
            score: plan.threshold,
            render_order: Some(i64::MAX),
            status: "injected",
            drop_reason: None,
            title: "Relevance".to_string(),
            provenance: plan.provenance(),
            staleness: format!("policy={SESSIONSTART_RELEVANCE_POLICY_VERSION}"),
        }
    }

    fn memory_item(
        memory: &Memory,
        channel: &'static str,
        render_order: Option<i64>,
        status: &'static str,
        drop_reason: Option<&'static str>,
    ) -> Self {
        Self {
            item_kind: "memory",
            item_id: Some(memory.id),
            memory_id: Some(memory.id),
            channel,
            score: None,
            render_order,
            status,
            drop_reason,
            title: memory.title.clone(),
            provenance: memory_provenance(memory),
            staleness: memory_staleness(memory, chrono::Utc::now().timestamp()),
        }
    }
}

pub(in crate::context) fn memory_render_metadata_with_labels(
    memory: &Memory,
    now_epoch: i64,
    staleness_labels: &HashMap<i64, MemoryStalenessLabel>,
) -> String {
    format!(
        "src=memory:#{};{}",
        memory.id,
        memory_staleness_with_labels(memory, now_epoch, staleness_labels).replace("; ", ";")
    )
}

pub(in crate::context) fn memory_provenance(memory: &Memory) -> String {
    let mut parts = vec![format!("src=memory:#{}", memory.id)];
    if let Some(session_id) = memory
        .session_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        parts.push(format!("session={session_id}"));
    }
    parts.push(format!("scope={}", memory.scope));
    parts.join("; ")
}

pub(in crate::context) fn memory_staleness(memory: &Memory, now_epoch: i64) -> String {
    shared_memory_staleness(memory, now_epoch)
}

fn memory_staleness_with_labels(
    memory: &Memory,
    now_epoch: i64,
    staleness_labels: &HashMap<i64, MemoryStalenessLabel>,
) -> String {
    staleness_labels
        .get(&memory.id)
        .map(|label| label.label.clone())
        .unwrap_or_else(|| memory_staleness(memory, now_epoch))
}

pub(in crate::context) fn record_context_injection_items(
    conn: &rusqlite::Connection,
    invocation: &ContextInvocation,
    decision: &ContextGateDecision,
    rendered_items: &[ContextAuditItem],
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let key = decision
        .key
        .clone()
        .unwrap_or_else(|| super::injection_gate::injection_key_for_audit(invocation));
    let context_hash = decision.context_hash.as_deref();
    let output_mode = decision.output_mode.unwrap_or(match decision.action {
        ContextGateAction::Suppressed => "suppressed",
        ContextGateAction::Bypassed => "bypassed",
        ContextGateAction::FailOpen => "fail_open",
        ContextGateAction::EmittedFull => "full",
        ContextGateAction::EmittedDelta => "delta",
    });
    let run_id = format!(
        "{}:{}:{}",
        key,
        now,
        context_hash.unwrap_or(decision.reason)
    );

    let mut statement = conn.prepare(
        "INSERT INTO context_injection_items
         (injection_run_id, host, project, session_id, injection_key, hook_source,
          context_hash, output_mode, decision, item_kind, item_id, memory_id, channel,
          score, render_order, status, drop_reason, title, provenance, staleness,
          injected_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                 ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
    )?;
    for item in finalize_items_for_decision(decision, rendered_items) {
        statement.execute(params![
            run_id,
            invocation.host.as_env_value(),
            invocation.project,
            invocation.session_id,
            key,
            invocation.source,
            context_hash,
            output_mode,
            decision.reason,
            item.item_kind,
            item.item_id,
            item.memory_id,
            item.channel,
            item.score,
            item.render_order,
            item.status,
            item.drop_reason,
            item.title,
            item.provenance,
            item.staleness,
            now,
        ])?;
    }
    Ok(())
}

pub(in crate::context) fn build_context_audit_items(
    loaded: &LoadedContext,
    core_ids: &[i64],
    index_ids: &[i64],
    lesson_ids: &[i64],
    session_ids: &[i64],
    workstream_ids: &[i64],
    relevance: &SessionStartRelevancePlan,
) -> Vec<ContextAuditItem> {
    let mut items = vec![ContextAuditItem::relevance_policy(relevance)];
    let mut render_order = 1_i64;
    if loaded.memory_abstained {
        items.push(ContextAuditItem::abstained_memory("no_relevant_context"));
    }
    let core = core_ids.iter().copied().collect::<HashSet<_>>();
    let index = index_ids.iter().copied().collect::<HashSet<_>>();
    for id in core_ids {
        if let Some(memory) = loaded.memories.iter().find(|memory| memory.id == *id) {
            items.push(ContextAuditItem::injected_memory_with_labels(
                memory,
                "core",
                render_order,
                &loaded.staleness_labels,
            ));
            render_order += 1;
        }
    }
    for id in index_ids {
        if let Some(memory) = loaded.memories.iter().find(|memory| memory.id == *id) {
            let mut item = ContextAuditItem::injected_memory_with_labels(
                memory,
                "index",
                render_order,
                &loaded.staleness_labels,
            );
            if let Some(decision) = relevance.decision(&memory_stable_key(memory.id)) {
                item = item.with_score(decision.score);
            }
            items.push(item);
            render_order += 1;
        }
    }
    for memory in &loaded.memories {
        if !core.contains(&memory.id) && !index.contains(&memory.id) {
            let decision = relevance.decision(&memory_stable_key(memory.id));
            let reason = decision
                .and_then(|decision| decision.drop_reason)
                .unwrap_or("section_budget");
            let mut item = ContextAuditItem::dropped_memory(memory, "index", reason);
            if let Some(decision) = decision {
                item = item.with_score(decision.score);
            }
            items.push(item);
        }
    }
    let lesson = lesson_ids.iter().copied().collect::<HashSet<_>>();
    for id in lesson_ids {
        if let Some(lesson_memory) = loaded.lessons.iter().find(|lesson| lesson.memory.id == *id) {
            let mut item = ContextAuditItem::injected_memory_with_labels(
                &lesson_memory.memory,
                "lessons",
                render_order,
                &loaded.staleness_labels,
            );
            if let Some(decision) = relevance.decision(&memory_stable_key(lesson_memory.memory.id))
            {
                item = item.with_score(decision.score);
            }
            items.push(item);
            render_order += 1;
        }
    }
    for lesson_memory in &loaded.lessons {
        if !lesson.contains(&lesson_memory.memory.id) {
            let decision = relevance.decision(&memory_stable_key(lesson_memory.memory.id));
            let reason = decision
                .and_then(|decision| decision.drop_reason)
                .unwrap_or("section_budget");
            let mut item =
                ContextAuditItem::dropped_memory(&lesson_memory.memory, "lessons", reason);
            if let Some(decision) = decision {
                item = item.with_score(decision.score);
            }
            items.push(item);
        }
    }
    let sessions = session_ids.iter().copied().collect::<HashSet<_>>();
    for summary in &loaded.summaries {
        let decision = relevance.decision(&session_stable_key(summary.id));
        let score = decision.map_or(0.0, |decision| decision.score);
        if sessions.contains(&summary.id) {
            items.push(ContextAuditItem::session_summary(
                summary,
                Some(render_order),
                "injected",
                None,
                score,
            ));
            render_order += 1;
        } else {
            let reason = decision
                .and_then(|decision| decision.drop_reason)
                .unwrap_or("section_budget");
            items.push(ContextAuditItem::session_summary(
                summary,
                None,
                "dropped",
                Some(reason),
                score,
            ));
        }
    }
    let workstream = workstream_ids.iter().copied().collect::<HashSet<_>>();
    for id in workstream_ids {
        if let Some(item) = loaded.workstreams.iter().find(|item| item.id == *id) {
            items.push(ContextAuditItem::injected_workstream(
                item.id,
                &item.title,
                render_order,
                item.updated_at_epoch,
            ));
            render_order += 1;
        }
    }
    for item in &loaded.workstreams {
        if !workstream.contains(&item.id) {
            items.push(ContextAuditItem::dropped_workstream(
                item.id,
                &item.title,
                item.updated_at_epoch,
                "section_budget",
            ));
        }
    }
    items
}

fn finalize_items_for_decision(
    decision: &ContextGateDecision,
    rendered_items: &[ContextAuditItem],
) -> Vec<ContextAuditItem> {
    rendered_items
        .iter()
        .cloned()
        .map(|mut item| {
            if item.status == "injected" && !decision.output.contains(&item.title) {
                item.status = "dropped";
                item.render_order = None;
                item.drop_reason = Some(match decision.action {
                    ContextGateAction::Suppressed => "gate_suppressed",
                    ContextGateAction::EmittedDelta => "delta_preview",
                    _ => "total_char_limit",
                });
            }
            item
        })
        .collect()
}

pub(in crate::context) fn final_governed_injected_count(
    output: &str,
    rendered_items: &[ContextAuditItem],
) -> usize {
    rendered_items
        .iter()
        .filter(|item| {
            item.status == "injected"
                && matches!(item.channel, "lessons" | "index" | "sessions")
                && output.contains(&item.title)
        })
        .count()
}
