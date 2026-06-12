use anyhow::Result;
use rusqlite::params;
use std::collections::HashSet;

use crate::memory::{age_staleness_label, memory_staleness as shared_memory_staleness, Memory};

use super::injection_gate::{ContextGateAction, ContextGateDecision};
use super::invocation::ContextInvocation;
use super::types::LoadedContext;

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

    pub fn dropped_memory(memory: &Memory, channel: &'static str, reason: &'static str) -> Self {
        Self::memory_item(memory, channel, None, "dropped", Some(reason))
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

pub(in crate::context) fn memory_render_metadata(memory: &Memory, now_epoch: i64) -> String {
    format!(
        "src=memory:#{}; {}",
        memory.id,
        memory_staleness(memory, now_epoch)
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
    workstream_ids: &[i64],
) -> Vec<ContextAuditItem> {
    let mut items = Vec::new();
    let mut render_order = 1_i64;
    if loaded.memory_abstained {
        items.push(ContextAuditItem::abstained_memory("no_relevant_context"));
    }
    let core = core_ids.iter().copied().collect::<HashSet<_>>();
    let index = index_ids.iter().copied().collect::<HashSet<_>>();
    for id in core_ids {
        if let Some(memory) = loaded.memories.iter().find(|memory| memory.id == *id) {
            items.push(ContextAuditItem::injected_memory(
                memory,
                "core",
                render_order,
            ));
            render_order += 1;
        }
    }
    for id in index_ids {
        if let Some(memory) = loaded.memories.iter().find(|memory| memory.id == *id) {
            items.push(ContextAuditItem::injected_memory(
                memory,
                "index",
                render_order,
            ));
            render_order += 1;
        }
    }
    for memory in &loaded.memories {
        if !core.contains(&memory.id) && !index.contains(&memory.id) {
            items.push(ContextAuditItem::dropped_memory(
                memory,
                "memory",
                "section_budget",
            ));
        }
    }
    let lesson = lesson_ids.iter().copied().collect::<HashSet<_>>();
    for id in lesson_ids {
        if let Some(lesson_memory) = loaded.lessons.iter().find(|lesson| lesson.memory.id == *id) {
            items.push(ContextAuditItem::injected_memory(
                &lesson_memory.memory,
                "lessons",
                render_order,
            ));
            render_order += 1;
        }
    }
    for lesson_memory in &loaded.lessons {
        if !lesson.contains(&lesson_memory.memory.id) {
            items.push(ContextAuditItem::dropped_memory(
                &lesson_memory.memory,
                "lessons",
                "section_budget",
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
