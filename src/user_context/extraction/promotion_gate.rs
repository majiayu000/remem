use crate::runtime_config::AutoPromotePolicy;

use super::{CandidateSourceBatch, ParsedUserContextCandidate};

pub(super) fn is_auto_promote_allowed(
    candidate: &ParsedUserContextCandidate,
    batch: &CandidateSourceBatch,
    policy: &AutoPromotePolicy,
) -> bool {
    matches!(
        candidate.claim_type,
        super::super::claims::UserContextClaimType::Preference
            | super::super::claims::UserContextClaimType::Constraint
    ) && candidate.risk_class == super::super::candidates::UserContextCandidateRisk::Low
        && candidate.sensitivity == super::super::claims::UserContextSensitivity::Normal
        && candidate.confidence >= policy.min_confidence
        && policy.allows_source_kind(&candidate.source_kind)
        && !super::requires_third_party_framing(candidate)
        && candidate
            .source_event_ids
            .iter()
            .all(|id| batch.event_is_user_authored(*id))
        && (!policy.require_text_support
            || super::is_supported_by_user_source_event(candidate, batch))
}

pub(super) fn blocked_reason(
    candidate: &ParsedUserContextCandidate,
    batch: &CandidateSourceBatch,
    policy: &AutoPromotePolicy,
) -> &'static str {
    if super::requires_third_party_framing(candidate) {
        return "third_party_requires_review";
    }
    if !matches!(
        candidate.claim_type,
        super::super::claims::UserContextClaimType::Preference
            | super::super::claims::UserContextClaimType::Constraint
    ) {
        return "claim_type_requires_review";
    }
    if candidate.risk_class != super::super::candidates::UserContextCandidateRisk::Low {
        return "risk_requires_review";
    }
    if candidate.sensitivity != super::super::claims::UserContextSensitivity::Normal {
        return "sensitivity_requires_review";
    }
    if candidate.confidence < policy.min_confidence {
        return "low_confidence";
    }
    if !policy.allows_source_kind(&candidate.source_kind) {
        return "source_requires_review";
    }
    if !candidate
        .source_event_ids
        .iter()
        .all(|id| batch.event_is_user_authored(*id))
    {
        return "source_not_user_authored";
    }
    if policy.require_text_support && !super::is_supported_by_user_source_event(candidate, batch) {
        return "no_supporting_source_event";
    }
    "requires_review"
}
