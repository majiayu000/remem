//! Deterministic CurrentTruth resolution policy (GH933 Phase A).
//!
//! Strict order, first hit wins: scope match -> as-of filter -> explicit
//! supersedes -> evidence-trust tier -> recency. Refutes relations and
//! unbreakable ties surface as `Contradicted`; empty groups abstain as
//! `Unknown`. No randomness, no LLM, no stored confidence in the decision
//! path.

use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::Connection;

use super::adapter::{load_memory_claim_groups_at, load_user_claim_groups, reference_epoch};
use super::lifecycle::apply_expiry;
use super::types::{
    ClaimRelationKind, ClaimView, CurrentTruthProjection, CurrentTruthView, EvidenceTrust,
    PublicationState, RelationView, RetentionState, TruthQuery, TruthSelectionReason,
    ValidityState, Visibility, TRUTH_PROJECTION_VERSION,
};

/// Project current truth for one project/branch scope from memory-backed
/// claims and their relations.
pub fn project_current_truth(
    conn: &Connection,
    query: &TruthQuery,
) -> Result<CurrentTruthProjection> {
    project_current_truth_at_reference_epoch(conn, query, reference_epoch(query))
}

pub(crate) fn project_current_truth_at_reference_epoch(
    conn: &Connection,
    query: &TruthQuery,
    reference_epoch: i64,
) -> Result<CurrentTruthProjection> {
    let (claims, relations) = load_memory_claim_groups_at(conn, query, reference_epoch)?;
    Ok(resolve_projection(
        query,
        claims,
        relations,
        reference_epoch,
    ))
}

/// Project current truth for one user-context owner. `query.project` is
/// unused for row selection here; the output scope echoes the owner key.
pub fn project_user_claim_truth(
    conn: &Connection,
    owner_scope: &str,
    owner_key: &str,
    as_of_epoch: Option<i64>,
) -> Result<CurrentTruthProjection> {
    let (claims, relations) = load_user_claim_groups(conn, owner_scope, owner_key)?;
    let query = TruthQuery {
        project: owner_key.to_string(),
        branch: None,
        as_of_epoch,
        subject_key: None,
    };
    Ok(resolve_projection(
        &query,
        claims,
        relations,
        reference_epoch(&query),
    ))
}

fn resolve_projection(
    query: &TruthQuery,
    claims: Vec<ClaimView>,
    relations: Vec<RelationView>,
    reference_epoch: i64,
) -> CurrentTruthProjection {
    let mut groups: BTreeMap<String, Vec<ClaimView>> = BTreeMap::new();
    for claim in claims {
        if let Some(subject) = &query.subject_key {
            if &claim.subject_key != subject {
                continue;
            }
        }
        groups
            .entry(claim.subject_key.clone())
            .or_default()
            .push(claim);
    }
    let truths = groups
        .into_iter()
        .map(|(subject_key, group)| resolve_group(subject_key, group, &relations, reference_epoch))
        .collect();
    CurrentTruthProjection {
        projection_version: TRUTH_PROJECTION_VERSION,
        project: query.project.clone(),
        branch: query.branch.clone(),
        as_of_epoch: query.as_of_epoch,
        truths,
    }
}

fn relation_effective(relation: &RelationView, reference_epoch: i64) -> bool {
    if relation.created_at_epoch > reference_epoch {
        return false;
    }
    if let Some(from) = relation.valid_from_epoch {
        if from > reference_epoch {
            return false;
        }
    }
    if let Some(to) = relation.valid_to_epoch {
        if to <= reference_epoch {
            return false;
        }
    }
    true
}

/// A claim is eligible as current truth only when it is published, live,
/// visible, and valid at the reference time.
fn eligible(claim: &ClaimView, reference_epoch: i64) -> bool {
    if claim.created_at_epoch > reference_epoch {
        return false;
    }
    if let Some(from) = claim.valid_from_epoch {
        if from > reference_epoch {
            return false;
        }
    }
    let lifecycle = apply_expiry(claim.lifecycle, claim.valid_to_epoch, reference_epoch);
    lifecycle.publication == PublicationState::Active
        && lifecycle.retention == RetentionState::Live
        && lifecycle.visibility == Visibility::Visible
        && lifecycle.validity == ValidityState::Current
}

fn claim_trust_tier(claim: &ClaimView) -> EvidenceTrust {
    claim
        .evidence
        .iter()
        .map(|evidence| evidence.trust)
        .max()
        .unwrap_or(EvidenceTrust::ModelGenerated)
}

fn abstention(subject_key: String, rejected: Vec<String>) -> CurrentTruthView {
    CurrentTruthView {
        subject_key,
        claim: None,
        validity: ValidityState::Unknown,
        evidence: Vec::new(),
        supporting_relations: Vec::new(),
        contradicting_relations: Vec::new(),
        rejected,
        conflicting_claims: Vec::new(),
        selected_reason: TruthSelectionReason::InsufficientEvidence,
    }
}

fn resolve_group(
    subject_key: String,
    group: Vec<ClaimView>,
    relations: &[RelationView],
    reference_epoch: i64,
) -> CurrentTruthView {
    let mut rejected: Vec<String> = Vec::new();
    let mut survivors: Vec<ClaimView> = Vec::new();
    for claim in group {
        if eligible(&claim, reference_epoch) {
            survivors.push(claim);
        } else {
            rejected.push(claim.canonical_ref);
        }
    }
    let group_refs: Vec<String> = survivors
        .iter()
        .map(|claim| claim.canonical_ref.clone())
        .chain(rejected.iter().cloned())
        .collect();
    let group_relations: Vec<&RelationView> = relations
        .iter()
        .filter(|relation| {
            relation_effective(relation, reference_epoch)
                && (group_refs.contains(&relation.from_ref)
                    || group_refs.contains(&relation.to_ref))
        })
        .collect();

    // Explicit supersedes beats recency: drop superseded survivors.
    let superseded_by_relation: Vec<String> = group_relations
        .iter()
        .filter(|relation| relation.kind == ClaimRelationKind::Supersedes)
        .map(|relation| relation.to_ref.clone())
        .collect();
    let mut supersedes_applied = false;
    survivors.retain(|claim| {
        if superseded_by_relation.contains(&claim.canonical_ref) {
            supersedes_applied = true;
            rejected.push(claim.canonical_ref.clone());
            false
        } else {
            true
        }
    });

    if survivors.is_empty() {
        return abstention(subject_key, rejected);
    }

    // A refutes relation between two survivors is an unresolved conflict:
    // never silently pick a side.
    let survivor_refs: Vec<&str> = survivors
        .iter()
        .map(|claim| claim.canonical_ref.as_str())
        .collect();
    let refutes: Vec<RelationView> = group_relations
        .iter()
        .filter(|relation| {
            relation.kind == ClaimRelationKind::Refutes
                && survivor_refs.contains(&relation.from_ref.as_str())
                && survivor_refs.contains(&relation.to_ref.as_str())
        })
        .map(|relation| (*relation).clone())
        .collect();
    if !refutes.is_empty() {
        return CurrentTruthView {
            subject_key,
            claim: None,
            validity: ValidityState::Contradicted,
            evidence: survivors
                .iter()
                .flat_map(|claim| claim.evidence.clone())
                .collect(),
            supporting_relations: Vec::new(),
            contradicting_relations: refutes,
            rejected,
            conflicting_claims: survivors,
            selected_reason: TruthSelectionReason::UnresolvedConflict,
        };
    }

    let reason = if survivors.len() == 1 {
        if supersedes_applied {
            TruthSelectionReason::ExplicitSupersedes
        } else {
            TruthSelectionReason::OnlySurvivingClaim
        }
    } else {
        // Verified evidence beats model-generated; only then recency.
        let best_tier = survivors
            .iter()
            .map(claim_trust_tier)
            .max()
            .unwrap_or(EvidenceTrust::ModelGenerated);
        let top_tier_count = survivors
            .iter()
            .filter(|claim| claim_trust_tier(claim) == best_tier)
            .count();
        if top_tier_count == 1 {
            survivors.retain(|claim| {
                let keep = claim_trust_tier(claim) == best_tier;
                if !keep {
                    rejected.push(claim.canonical_ref.clone());
                }
                keep
            });
            TruthSelectionReason::VerifiedEvidencePreferred
        } else {
            survivors.retain(|claim| {
                let keep = claim_trust_tier(claim) == best_tier;
                if !keep {
                    rejected.push(claim.canonical_ref.clone());
                }
                keep
            });
            let newest = survivors
                .iter()
                .map(|claim| claim.updated_at_epoch)
                .max()
                .unwrap_or(0);
            let newest_count = survivors
                .iter()
                .filter(|claim| claim.updated_at_epoch == newest)
                .count();
            if newest_count > 1 {
                // Same tier, same timestamp: no deterministic winner exists.
                return CurrentTruthView {
                    subject_key,
                    claim: None,
                    validity: ValidityState::Contradicted,
                    evidence: survivors
                        .iter()
                        .flat_map(|claim| claim.evidence.clone())
                        .collect(),
                    supporting_relations: Vec::new(),
                    contradicting_relations: Vec::new(),
                    rejected,
                    conflicting_claims: survivors,
                    selected_reason: TruthSelectionReason::UnresolvedConflict,
                };
            }
            survivors.retain(|claim| {
                let keep = claim.updated_at_epoch == newest;
                if !keep {
                    rejected.push(claim.canonical_ref.clone());
                }
                keep
            });
            TruthSelectionReason::MostRecent
        }
    };

    let winner = survivors.remove(0);
    let supporting_relations: Vec<RelationView> = group_relations
        .iter()
        .filter(|relation| {
            (relation.from_ref == winner.canonical_ref || relation.to_ref == winner.canonical_ref)
                && matches!(
                    relation.kind,
                    ClaimRelationKind::Supports
                        | ClaimRelationKind::Supersedes
                        | ClaimRelationKind::DerivedFrom
                )
        })
        .map(|relation| (*relation).clone())
        .collect();
    CurrentTruthView {
        subject_key,
        evidence: winner.evidence.clone(),
        validity: ValidityState::Current,
        supporting_relations,
        contradicting_relations: Vec::new(),
        rejected,
        conflicting_claims: Vec::new(),
        selected_reason: reason,
        claim: Some(winner),
    }
}
