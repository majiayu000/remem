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
use rusqlite::{Connection, OptionalExtension};

use crate::context_bundle::{ChannelKind, ContextItem, ItemValidity, SourceKind, TrustClass};
use crate::memory::{Memory, MemoryStalenessLabel, MemoryType};
use std::collections::HashSet;

use super::poisoning::PoisoningDrops;
use super::policy::{ContextLimits, ContextPolicy};
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
#[cfg(test)]
pub(crate) fn load_session_start_candidates(
    conn: &Connection,
    project: &str,
    cwd: &str,
    current_branch: Option<&str>,
) -> Result<Vec<ContextItem>> {
    let limits = ContextLimits::from_env();
    Ok(
        load_session_start_candidates_with_limits(conn, project, cwd, current_branch, &limits)?
            .candidates,
    )
}

#[derive(Debug)]
pub(crate) struct LoadedBundleCandidates {
    pub(crate) candidates: Vec<ContextItem>,
    pub(crate) poisoning_drops: Vec<ContextItem>,
}

/// Load candidates against the same already-resolved limits that were hashed
/// into the retrieval plan. This prevents environment overrides from changing
/// the wire result while leaving the plan hash unchanged.
pub(crate) fn load_session_start_candidates_with_limits(
    conn: &Connection,
    project: &str,
    cwd: &str,
    current_branch: Option<&str>,
    limits: &ContextLimits,
) -> Result<LoadedBundleCandidates> {
    let policy = ContextPolicy::from_limits(*limits);
    let mut loaded = load_context_data_with_policy(conn, project, current_branch, &policy, false);
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
    let poisoning_drops = super::poisoning::drop_unacknowledged_poisoned_context(conn, &mut loaded);
    let mut discarded_core = String::new();
    let core = super::sections::render_core_memory_with_limits_and_staleness(
        &mut discarded_core,
        &loaded.memories,
        &policy.limits,
        loaded.render_reference_epoch,
        &loaded.staleness_labels,
    );
    let core_ids = core.ids.into_iter().collect::<HashSet<_>>();
    let mut items = candidates_from_loaded(&loaded, project, &core_ids);
    let (preferences, preference_poisoning_drops) =
        preference_candidates(conn, project, cwd, &policy)?;
    items.extend(preferences);
    apply_persisted_memory_trust(conn, &mut items)?;
    Ok(LoadedBundleCandidates {
        candidates: items,
        poisoning_drops: poisoning_drop_candidates(
            poisoning_drops,
            preference_poisoning_drops,
            project,
        ),
    })
}

/// `Memory` predates source-trust provenance and intentionally does not expose
/// that database-only column. The bundle still has to honor the retrieval
/// plan's trust floor, so enrich memory candidates from the canonical row:
/// direct user-authored saves are trusted; extracted/tool/repo/external rows
/// remain standard unless the poisoning gate quarantined them separately.
fn apply_persisted_memory_trust(conn: &Connection, items: &mut [ContextItem]) -> Result<()> {
    for item in items {
        let Some(memory_id) = item
            .stable_key
            .strip_prefix("memory:")
            .and_then(|value| value.parse::<i64>().ok())
        else {
            continue;
        };
        let source_trust: Option<String> = conn
            .query_row(
                "SELECT source_trust_class FROM memories WHERE id = ?1",
                [memory_id],
                |row| row.get(0),
            )
            .optional()?;
        if source_trust.as_deref() == Some("user_prompt") {
            item.trust = TrustClass::Trusted;
        }
    }
    Ok(())
}

fn poisoning_drop_candidates(
    drops: PoisoningDrops,
    preference_drops: Vec<Memory>,
    project: &str,
) -> Vec<ContextItem> {
    let mut items = Vec::new();
    for memory in drops.memories {
        let channel = MemoryType::parse(&memory.memory_type)
            .filter(|memory_type| memory_type.is_core())
            .map_or(ChannelKind::MemoryIndex, |_| ChannelKind::Core);
        items.push(redact_poisoned(bundle_memory_item(
            &memory, channel, None, project,
        )));
    }
    for memory in drops.lessons {
        items.push(redact_poisoned(bundle_memory_item(
            &memory,
            ChannelKind::Lessons,
            None,
            project,
        )));
    }
    for memory in preference_drops {
        items.push(redact_poisoned(bundle_memory_item(
            &memory,
            ChannelKind::Preferences,
            None,
            project,
        )));
    }
    for summary in drops.summaries {
        items.push(redact_poisoned(summary_item(&summary, project)));
    }
    for workstream in drops.workstreams {
        items.push(redact_poisoned(workstream_item(&workstream)));
    }
    items
}

fn redact_poisoned(mut item: ContextItem) -> ContextItem {
    item.title.clear();
    item.text.clear();
    item.trust = TrustClass::Quarantined;
    item
}

/// Convert an already-loaded and poisoning-filtered SessionStart snapshot to
/// bundle candidates without reading the canonical sections a second time.
/// `core_ids` comes from the compatibility core renderer: core memories that
/// did not win a core slot remain eligible for the memory index, matching the
/// established SessionStart behavior.
pub(super) fn session_start_candidates_from_loaded(
    loaded: &LoadedContext,
    project: &str,
    preferences: &[Memory],
    core_ids: &HashSet<i64>,
) -> Result<Vec<ContextItem>> {
    if !loaded.errors.is_empty() {
        bail!(
            "canonical context load failed for sections [{}]",
            loaded
                .errors
                .iter()
                .map(|error| error.section)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let mut items = candidates_from_loaded(loaded, project, core_ids);
    items.extend(ordered_preference_candidates(preferences, project));
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
) -> Result<(Vec<ContextItem>, Vec<Memory>)> {
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
    Ok((
        ordered_preference_candidates(&details.rendered_memories, project),
        details.poisoning_drops,
    ))
}

fn ordered_preference_candidates(selected: &[Memory], project: &str) -> Vec<ContextItem> {
    selected
        .iter()
        .map(|memory| {
            let mut item = bundle_memory_item(memory, ChannelKind::Preferences, None, project);
            // The canonical preference selector is deliberately branch
            // agnostic. Once it admitted a preference for SessionStart, do
            // not let the generic bundle scope check impose a new branch
            // filter that the byte-compatible renderer never applied.
            item.branch = None;
            item
        })
        .collect()
}

fn candidates_from_loaded(
    loaded: &LoadedContext,
    project: &str,
    core_ids: &HashSet<i64>,
) -> Vec<ContextItem> {
    let mut items = Vec::new();
    for memory in loaded.memories.iter().filter(|memory| {
        core_ids.contains(&memory.id)
            || MemoryType::parse(&memory.memory_type).is_none_or(MemoryType::is_indexed)
    }) {
        let label = loaded.staleness_labels.get(&memory.id);
        items.push(bundle_memory_item(
            memory,
            memory_channel(memory, core_ids),
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
        items.push(workstream_item(workstream));
    }
    for summary in &loaded.summaries {
        items.push(summary_item(summary, project));
    }
    items
}

fn workstream_item(workstream: &crate::workstream::WorkStream) -> ContextItem {
    ContextItem {
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
    }
}

/// Core types feed current truth; everything else lands in the index.
/// Preferences never appear here — they arrive through
/// [`preference_candidates`].
fn memory_channel(memory: &Memory, core_ids: &HashSet<i64>) -> ChannelKind {
    if core_ids.contains(&memory.id) {
        ChannelKind::Core
    } else {
        ChannelKind::MemoryIndex
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
        // `project` is the effective SessionStart scope. Global overlays and
        // legacy rows may carry a different storage project, but they reached
        // this function only after the canonical ownership selector admitted
        // them for the requested project.
        project: Some(project.to_string()),
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
