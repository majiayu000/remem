//! CurrentTruth on the compile path: annotate selected claims, audit a
//! Core-vs-projection shadow, then activate the `current_truth` channel
//! (selected claims plus abstentions; no Core newest-wins).

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::Memory;
use crate::truth::{
    project_current_truth, CurrentTruthProjection, CurrentTruthView, TruthQuery,
    TruthSelectionReason, ValidityState, TRUTH_PROJECTION_VERSION,
};

use super::domain::{
    AuditEntry, ChannelKind, ContextBundle, ContextItem, CurrentTruthShadowDiff, ItemValidity,
    SourceKind, TrustClass,
};

const MAX_EVIDENCE_REFS: usize = 32;
const MAX_EMITTED_ABSTENTIONS: usize = 32;
pub(super) const SHADOW_CORE_ONLY: &str = "core_only";
pub(super) const SHADOW_PROJECTION_ONLY: &str = "projection_only";
pub(super) const SHADOW_ABSTAINED: &str = "abstained";
pub(crate) const REASON_NOT_CURRENT_TRUTH_CLAIM: &str = "not_current_truth_claim";

/// Stable projection identity already used by doctor/truth grouping.
pub(super) fn projection_ref_for(subject_key: &str) -> String {
    format!("current_truth:v{TRUTH_PROJECTION_VERSION}:{subject_key}")
}

pub(crate) fn project_for_scope(
    conn: &Connection,
    project: &str,
    branch: Option<&str>,
    as_of_epoch: i64,
) -> Result<CurrentTruthProjection> {
    project_current_truth(
        conn,
        &TruthQuery {
            project: project.to_string(),
            branch: branch.map(str::to_string),
            // MCP v1 treats 0 as "now"; a positive pin is a real as-of.
            as_of_epoch: (as_of_epoch > 0).then_some(as_of_epoch),
            subject_key: None,
        },
    )
}

pub(crate) fn try_project_for_scope(
    conn: &Connection,
    project: &str,
    branch: Option<&str>,
    as_of_epoch: i64,
    log_component: &str,
) -> Option<CurrentTruthProjection> {
    match project_for_scope(conn, project, branch, as_of_epoch) {
        Ok(projection) => Some(projection),
        Err(error) => {
            crate::log::error(
                log_component,
                &format!("current truth projection failed for {project}: {error}"),
            );
            None
        }
    }
}

pub(crate) fn selected_memory_ids(projection: &CurrentTruthProjection) -> HashSet<i64> {
    projection
        .truths
        .iter()
        .filter_map(|truth| {
            truth
                .claim
                .as_ref()
                .and_then(|claim| memory_id_from_ref(&claim.canonical_ref))
        })
        .collect()
}

pub(crate) fn abstained_memory_ids(projection: &CurrentTruthProjection) -> HashSet<i64> {
    projection
        .truths
        .iter()
        .filter(|truth| truth.claim.is_none())
        .filter(|truth| truth.validity == ValidityState::Contradicted)
        .flat_map(|truth| {
            abstain_claim_refs(truth)
                .into_iter()
                .filter_map(|claim_ref| memory_id_from_ref(&claim_ref))
        })
        .collect()
}

pub(crate) fn abstention_reason_for_memory(
    projection: &CurrentTruthProjection,
    memory_id: i64,
) -> Option<&'static str> {
    let needle = format!("memory:{memory_id}");
    projection.truths.iter().find_map(|truth| {
        if truth.claim.is_some() {
            return None;
        }
        if truth.validity != ValidityState::Contradicted {
            return None;
        }
        abstain_claim_refs(truth)
            .iter()
            .any(|claim_ref| claim_ref == &needle)
            .then_some(abstain_reason(truth.selected_reason))
    })
}

/// Core renderer input: selected CurrentTruth claims only. Projection load
/// failure stays fail-open (today's Core mapping).
pub(crate) fn core_render_memories<'a>(
    memories: &'a [Memory],
    projection: Option<&CurrentTruthProjection>,
) -> Cow<'a, [Memory]> {
    let Some(projection) = projection else {
        return Cow::Borrowed(memories);
    };
    let selected = selected_memory_ids(projection);
    Cow::Owned(
        memories
            .iter()
            .filter(|memory| selected.contains(&memory.id))
            .cloned()
            .collect(),
    )
}

pub(super) fn annotate_current_truth_items(
    items: &mut [ContextItem],
    projection: &CurrentTruthProjection,
) {
    let selected: HashMap<&str, &CurrentTruthView> = projection
        .truths
        .iter()
        .filter_map(|truth| {
            truth
                .claim
                .as_ref()
                .map(|claim| (claim.canonical_ref.as_str(), truth))
        })
        .collect();
    for item in items {
        let Some(truth) = selected.get(item.stable_key.as_str()).copied() else {
            continue;
        };
        item.projection_ref = Some(projection_ref_for(&truth.subject_key));
        item.evidence_refs = evidence_refs(truth);
    }
}

pub(super) fn attach_shadow_comparison(
    bundle: &mut ContextBundle,
    projection: &CurrentTruthProjection,
) {
    bundle.audit.shadow_comparison = shadow_diffs(&bundle.current_truth, projection);
}

/// After shadow: drop Core items that are not selected claims, then emit
/// compact abstention rows for contradicted/unknown subjects.
pub(super) fn activate_current_truth_channel(
    bundle: &mut ContextBundle,
    projection: &CurrentTruthProjection,
) {
    retain_selected_current_truth(bundle, projection);
    emit_abstention_items(bundle, projection);
}

pub(super) fn is_current_truth_abstention(item: &ContextItem) -> bool {
    item.source_kind == SourceKind::GraphDerived && item.stable_key.starts_with("current_truth:v")
}

pub(crate) fn append_core_abstention_lines(
    output: &mut String,
    projection: &CurrentTruthProjection,
    max_additional_chars: usize,
) -> usize {
    if max_additional_chars == 0 {
        return 0;
    }
    let mut added = 0usize;
    let mut remaining = max_additional_chars;
    if output.is_empty() {
        let header = "## Core\n";
        let header_chars = header.chars().count();
        if header_chars >= remaining {
            return 0;
        }
        output.push_str(header);
        added += header_chars;
        remaining -= header_chars;
    }
    for truth in abstained_truths(projection)
        .into_iter()
        .take(MAX_EMITTED_ABSTENTIONS)
    {
        let line = format!("{}\n", abstention_line(truth));
        let line_chars = line.chars().count();
        if line_chars > remaining {
            break;
        }
        output.push_str(&line);
        added += line_chars;
        remaining -= line_chars;
    }
    added
}

fn retain_selected_current_truth(bundle: &mut ContextBundle, projection: &CurrentTruthProjection) {
    let selected = selected_claim_keys(projection);
    let abstained = abstained_claim_key_set(projection);
    bundle.current_truth.retain(|item| {
        is_current_truth_abstention(item) || selected.contains(item.stable_key.as_str())
    });
    for entry in &mut bundle.audit.entries {
        if !entry.selected || entry.channel != ChannelKind::Core {
            continue;
        }
        if selected.contains(entry.stable_key.as_str())
            || entry.stable_key.starts_with("current_truth:v")
        {
            continue;
        }
        entry.selected = false;
        entry.reason = if abstained.contains(&entry.stable_key) {
            abstention_reason_for_claim(projection, &entry.stable_key)
                .unwrap_or("unresolved_conflict")
                .to_string()
        } else {
            REASON_NOT_CURRENT_TRUTH_CLAIM.to_string()
        };
    }
    recount_audit(bundle);
}

fn emit_abstention_items(bundle: &mut ContextBundle, projection: &CurrentTruthProjection) {
    let existing: HashSet<String> = bundle
        .current_truth
        .iter()
        .map(|item| item.stable_key.clone())
        .collect();
    for truth in abstained_truths(projection)
        .into_iter()
        .take(MAX_EMITTED_ABSTENTIONS)
    {
        let item = abstention_item(truth);
        if !existing.contains(&item.stable_key) {
            bundle
                .audit
                .entries
                .push(abstention_audit_entry(&item, truth));
            bundle.current_truth.push(item);
        }
    }
    bundle.audit.entries.sort_by(|left, right| {
        (left.channel, &left.stable_key).cmp(&(right.channel, &right.stable_key))
    });
    recount_audit(bundle);
}

fn abstention_item(truth: &CurrentTruthView) -> ContextItem {
    ContextItem {
        stable_key: projection_ref_for(&truth.subject_key),
        channel: ChannelKind::Core,
        title: format!("Abstained {}", truth.subject_key),
        text: abstention_line(truth),
        source_kind: SourceKind::GraphDerived,
        canonical_ref: None,
        projection_ref: Some(projection_ref_for(&truth.subject_key)),
        evidence_refs: Vec::new(),
        validity: ItemValidity::Current,
        trust: TrustClass::Standard,
        project: None,
        branch: None,
    }
}

fn abstention_audit_entry(item: &ContextItem, truth: &CurrentTruthView) -> AuditEntry {
    AuditEntry {
        stable_key: item.stable_key.clone(),
        channel: item.channel,
        source_kind: item.source_kind,
        validity: item.validity,
        selected: true,
        reason: abstain_reason(truth.selected_reason).to_string(),
        relevance_score: None,
        token_estimate: estimate_tokens(item),
    }
}

fn abstention_line(truth: &CurrentTruthView) -> String {
    format!(
        "Abstained {}: {} ({})",
        truth.subject_key,
        abstain_reason(truth.selected_reason),
        abstain_claim_refs(truth).join(", ")
    )
}

fn abstained_truths(projection: &CurrentTruthProjection) -> Vec<&CurrentTruthView> {
    let mut truths: Vec<&CurrentTruthView> = projection
        .truths
        .iter()
        .filter(|truth| truth.claim.is_none())
        .filter(|truth| truth.validity == ValidityState::Contradicted)
        .collect();
    truths.sort_by(|left, right| left.subject_key.cmp(&right.subject_key));
    truths
}

fn selected_claim_keys(projection: &CurrentTruthProjection) -> HashSet<&str> {
    projection
        .truths
        .iter()
        .filter_map(|truth| {
            truth
                .claim
                .as_ref()
                .map(|claim| claim.canonical_ref.as_str())
        })
        .collect()
}

fn abstained_claim_key_set(projection: &CurrentTruthProjection) -> HashSet<String> {
    projection
        .truths
        .iter()
        .filter(|truth| truth.claim.is_none())
        .filter(|truth| {
            matches!(
                truth.validity,
                ValidityState::Contradicted | ValidityState::Unknown
            )
        })
        .flat_map(abstain_claim_refs)
        .collect()
}

fn abstention_reason_for_claim(
    projection: &CurrentTruthProjection,
    claim_ref: &str,
) -> Option<&'static str> {
    projection.truths.iter().find_map(|truth| {
        if truth.claim.is_some() {
            return None;
        }
        abstain_claim_refs(truth)
            .iter()
            .any(|candidate| candidate == claim_ref)
            .then_some(abstain_reason(truth.selected_reason))
    })
}

fn recount_audit(bundle: &mut ContextBundle) {
    bundle.audit.candidates_considered = bundle.audit.entries.len() as u32;
    bundle.audit.selected_count = bundle
        .audit
        .entries
        .iter()
        .filter(|entry| entry.selected)
        .count() as u32;
    bundle.audit.dropped_count = bundle
        .audit
        .candidates_considered
        .saturating_sub(bundle.audit.selected_count);
    bundle.audit.token_estimate = bundle
        .audit
        .entries
        .iter()
        .filter(|entry| entry.selected)
        .map(|entry| entry.token_estimate)
        .sum();
}

fn estimate_tokens(item: &ContextItem) -> u32 {
    let separator = u32::from(!item.title.is_empty() && !item.text.is_empty());
    let chars = item.title.chars().count() as u32 + item.text.chars().count() as u32 + separator;
    chars.div_ceil(4)
}

fn memory_id_from_ref(canonical_ref: &str) -> Option<i64> {
    canonical_ref.strip_prefix("memory:")?.parse().ok()
}

fn evidence_refs(truth: &CurrentTruthView) -> Vec<String> {
    let mut refs: Vec<String> = truth
        .evidence
        .iter()
        .map(|evidence| evidence.evidence_ref.clone())
        .filter(|value| !value.is_empty())
        .collect();
    refs.sort();
    refs.dedup();
    refs.truncate(MAX_EVIDENCE_REFS);
    refs
}

fn shadow_diffs(
    core_items: &[ContextItem],
    projection: &CurrentTruthProjection,
) -> Vec<CurrentTruthShadowDiff> {
    let core_keys: BTreeSet<&str> = core_items
        .iter()
        .map(|item| item.stable_key.as_str())
        .collect();
    let mut selected_keys: BTreeSet<&str> = BTreeSet::new();
    let mut selected_by_key: HashMap<&str, &CurrentTruthView> = HashMap::new();
    let mut abstained_claim_keys: HashSet<String> = HashSet::new();
    let mut diffs = Vec::new();

    for truth in &projection.truths {
        if let Some(claim) = &truth.claim {
            selected_keys.insert(claim.canonical_ref.as_str());
            selected_by_key.insert(claim.canonical_ref.as_str(), truth);
            continue;
        }
        if !matches!(
            truth.validity,
            ValidityState::Contradicted | ValidityState::Unknown
        ) {
            continue;
        }
        let claim_refs = abstain_claim_refs(truth);
        abstained_claim_keys.extend(claim_refs.iter().cloned());
        diffs.push(CurrentTruthShadowDiff {
            stable_key: projection_ref_for(&truth.subject_key),
            verdict: SHADOW_ABSTAINED.to_string(),
            projection_ref: Some(projection_ref_for(&truth.subject_key)),
            claim_refs,
            reason: abstain_reason(truth.selected_reason).to_string(),
        });
    }

    for key in &core_keys {
        if selected_keys.contains(key) || abstained_claim_keys.contains(*key) {
            continue;
        }
        diffs.push(CurrentTruthShadowDiff {
            stable_key: (*key).to_string(),
            verdict: SHADOW_CORE_ONLY.to_string(),
            projection_ref: None,
            claim_refs: vec![(*key).to_string()],
            reason: "core_channel_not_selected_claim".to_string(),
        });
    }
    for key in &selected_keys {
        if core_keys.contains(key) {
            continue;
        }
        let projection_ref = selected_by_key
            .get(*key)
            .map(|truth| projection_ref_for(&truth.subject_key));
        diffs.push(CurrentTruthShadowDiff {
            stable_key: (*key).to_string(),
            verdict: SHADOW_PROJECTION_ONLY.to_string(),
            projection_ref,
            claim_refs: vec![(*key).to_string()],
            reason: "selected_claim_not_in_core".to_string(),
        });
    }

    diffs.sort_by(|left, right| {
        left.verdict
            .cmp(&right.verdict)
            .then_with(|| left.stable_key.cmp(&right.stable_key))
    });
    diffs
}

fn abstain_claim_refs(truth: &CurrentTruthView) -> Vec<String> {
    let mut refs: Vec<String> = truth
        .conflicting_claims
        .iter()
        .map(|claim| claim.canonical_ref.clone())
        .collect();
    if refs.is_empty() {
        refs = truth.rejected.clone();
    }
    refs.sort();
    refs.dedup();
    refs
}

fn abstain_reason(reason: TruthSelectionReason) -> &'static str {
    match reason {
        TruthSelectionReason::UnresolvedConflict => "unresolved_conflict",
        TruthSelectionReason::InsufficientEvidence => "insufficient_evidence",
        TruthSelectionReason::OnlySurvivingClaim => "only_surviving_claim",
        TruthSelectionReason::ExplicitSupersedes => "explicit_supersedes",
        TruthSelectionReason::VerifiedEvidencePreferred => "verified_evidence_preferred",
        TruthSelectionReason::MostRecent => "most_recent",
    }
}
