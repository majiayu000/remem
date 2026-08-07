//! Bridge from the SessionStart loaders to the Context Bundle candidate
//! contract (GH-932).
//!
//! The conversion lives inside `context` on purpose: `LoadedContext` and
//! the section policy types are module-private, and widening them to the
//! crate just to build candidates elsewhere would leak the whole loader
//! shape. The crate-visible surface is one function returning
//! [`ContextItem`]s.
//!
//! Fail-closed: if any section failed to load, this returns `Err` rather
//! than a shorter candidate list. A partial load is indistinguishable
//! downstream from "the project genuinely has little memory", so silently
//! degrading here would render a confident-looking bundle over missing
//! canonical data.

use anyhow::{bail, Result};
use rusqlite::Connection;

use crate::context_bundle::{ChannelKind, ContextItem, ItemValidity, SourceKind, TrustClass};
use crate::memory::{Memory, MemoryStalenessLabel, MemoryType};

use super::policy::ContextPolicy;
use super::query::load_context_data_with_policy;
use super::relevance::{memory_stable_key, session_stable_key};
use super::types::{LoadedContext, SessionSummaryBrief};

/// Memory status values that must never enter a bundle as trusted.
const STATUS_QUARANTINED: &str = "quarantined";
const STATUS_SUPERSEDED: &str = "superseded";

/// Load the SessionStart candidate set for `project` and convert it into
/// bundle candidates.
///
/// Returns `Err` when any section failed to load; the caller must turn
/// that into a `Blocked` bundle rather than executing over what survived.
pub(crate) fn load_session_start_candidates(
    conn: &Connection,
    project: &str,
    cwd: &str,
    current_branch: Option<&str>,
) -> Result<Vec<ContextItem>> {
    let policy = ContextPolicy::from_env();
    let loaded = load_context_data_with_policy(conn, project, current_branch, &policy, false);
    if !loaded.errors.is_empty() {
        let sections: Vec<&str> = loaded.errors.iter().map(|error| error.section).collect();
        bail!(
            "canonical context load failed for sections [{}]: {}",
            sections.join(", "),
            loaded
                .errors
                .iter()
                .map(|error| error.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    let mut items = candidates_from_loaded(&loaded, project);
    items.extend(preference_candidates(conn, project, cwd, &policy)?);
    Ok(items)
}

/// Preferences never reach `LoadedContext`: SessionStart selects and
/// renders them on their own path (scope split, CLAUDE.md dedup, own
/// limits). The bundle reuses that selection rather than re-deriving it,
/// then reads back the selected rows as canonical candidates.
fn preference_candidates(
    conn: &Connection,
    project: &str,
    cwd: &str,
    policy: &ContextPolicy,
) -> Result<Vec<ContextItem>> {
    let mut discarded_render = String::new();
    let limits = &policy.limits;
    let details = crate::memory::preference::render_preferences_with_context_details(
        &mut discarded_render,
        conn,
        project,
        cwd,
        limits.preference_project_limit,
        limits.preference_global_limit,
        limits.preference_char_limit,
    )?;
    let selected = crate::memory::get_memories_by_ids(conn, &details.rendered_ids, None)?;
    Ok(selected
        .iter()
        .map(|memory| bundle_memory_item(memory, ChannelKind::Preferences, None, project))
        .collect())
}

fn candidates_from_loaded(loaded: &LoadedContext, project: &str) -> Vec<ContextItem> {
    let mut items = Vec::new();
    for memory in &loaded.memories {
        let label = loaded.staleness_labels.get(&memory.id);
        items.push(bundle_memory_item(
            memory,
            memory_channel(memory),
            label,
            project,
        ));
    }
    for lesson in &loaded.lessons {
        let memory = &lesson.memory;
        let label = loaded.staleness_labels.get(&memory.id);
        items.push(bundle_memory_item(
            memory,
            ChannelKind::Lessons,
            label,
            project,
        ));
    }
    for workstream in &loaded.workstreams {
        items.push(ContextItem {
            stable_key: format!("workstream:{}", workstream.id),
            channel: ChannelKind::Workstreams,
            title: workstream.title.clone(),
            text: workstream_text(workstream),
            source_kind: SourceKind::Canonical,
            canonical_ref: Some(format!("workstream:{}", workstream.id)),
            projection_ref: None,
            evidence_refs: Vec::new(),
            validity: ItemValidity::Current,
            trust: TrustClass::Standard,
            project: Some(workstream.project.clone()),
            branch: None,
        });
    }
    for summary in &loaded.summaries {
        items.push(summary_item(summary, project));
    }
    items
}

/// Core types feed current truth; everything else lands in the index.
/// Preferences never appear here — they arrive through
/// [`preference_candidates`].
fn memory_channel(memory: &Memory) -> ChannelKind {
    match MemoryType::parse(&memory.memory_type) {
        Some(memory_type) if memory_type.is_core() => ChannelKind::Core,
        _ => ChannelKind::MemoryIndex,
    }
}

fn bundle_memory_item(
    memory: &Memory,
    channel: ChannelKind,
    label: Option<&MemoryStalenessLabel>,
    project: &str,
) -> ContextItem {
    ContextItem {
        stable_key: memory_stable_key(memory.id),
        channel,
        title: memory.title.clone(),
        text: memory.text.clone(),
        // SessionStart injects canonical memory rows only; generated
        // enrichment reaches retrieval through its own channel and must
        // not be relabeled canonical on the way into a bundle.
        source_kind: SourceKind::Canonical,
        canonical_ref: Some(memory_stable_key(memory.id)),
        projection_ref: None,
        evidence_refs: Vec::new(),
        validity: validity_for(memory, label),
        trust: trust_for(memory),
        project: Some(memory.project.clone()),
        branch: memory.branch.clone(),
    }
    .with_project_fallback(project)
}

fn summary_item(summary: &SessionSummaryBrief, project: &str) -> ContextItem {
    ContextItem {
        stable_key: session_stable_key(summary.id),
        channel: ChannelKind::Sessions,
        title: summary.request.clone(),
        text: summary.completed.clone().unwrap_or_default(),
        source_kind: SourceKind::Canonical,
        canonical_ref: Some(session_stable_key(summary.id)),
        projection_ref: None,
        evidence_refs: Vec::new(),
        validity: ItemValidity::Current,
        trust: TrustClass::Standard,
        project: Some(project.to_string()),
        branch: None,
    }
}

/// `superseded` status wins over any age label: a superseded claim is not
/// merely stale, and the plan's `include_superseded` filter must see it.
fn validity_for(memory: &Memory, label: Option<&MemoryStalenessLabel>) -> ItemValidity {
    if memory.status == STATUS_SUPERSEDED {
        return ItemValidity::Superseded;
    }
    match label.map(|label| label.age) {
        Some("old") | Some("aging") => ItemValidity::Stale,
        _ => ItemValidity::Current,
    }
}

fn trust_for(memory: &Memory) -> TrustClass {
    if memory.status == STATUS_QUARANTINED {
        TrustClass::Quarantined
    } else {
        TrustClass::Standard
    }
}

fn workstream_text(workstream: &crate::workstream::WorkStream) -> String {
    let mut text = workstream.title.clone();
    if let Some(next_action) = &workstream.next_action {
        text.push_str(" -> ");
        text.push_str(next_action);
    }
    if let Some(blockers) = &workstream.blockers {
        text.push_str(" (blockers: ");
        text.push_str(blockers);
        text.push(')');
    }
    text
}

impl ContextItem {
    /// Global-scope memories carry their own project string; anything with
    /// an empty project is attributed to the requested project so the
    /// executor's scope check has something concrete to compare.
    fn with_project_fallback(mut self, project: &str) -> Self {
        if self
            .project
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
        {
            self.project = Some(project.to_string());
        }
        self
    }
}
